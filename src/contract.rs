#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Addr, Binary, CosmosMsg, Deps, DepsMut, Empty, Env, MessageInfo, Order,
    Response, StdResult, WasmMsg,
};
use cw2::set_contract_version;
use cw721::msg::Cw721ExecuteMsg;
use cw721::receiver::Cw721ReceiveMsg;

use crate::error::ContractError;
use crate::msg::{
    AuctionResponse, AuctionsResponse, BidResponse, BidsResponse, ConfigResponse, ExecuteMsg,
    InstantiateMsg, PoolContentsResponse, PoolSizeResponse, QueryMsg, ReceiveNftAction,
    SwapStagingResponse,
};
use crate::state::{
    Auction, AuctionStatus, Bid, Config, ACTIVE_MEGA_AUCTIONS, AUCTIONS, BIDS, CONFIG, ESCROW,
    ESCROWED_BID_TOKENS, NEXT_AUCTION_ID, POOL, POOL_SIZE,
};

const CONTRACT_NAME: &str = "crates.io:mega-mad-scientist";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Default pagination limit
const DEFAULT_LIMIT: u32 = 30;
const MAX_LIMIT: u32 = 100;
const MAX_BIDDERS_PER_AUCTION_CAP: u64 = 1_000;
const MAX_STAGING_SIZE_CAP: u64 = 1_000;
const MAX_NFTS_PER_BID_CAP: u64 = 1_000;
type Cw721ExecEmpty = Cw721ExecuteMsg<Empty, Empty, Empty>;

struct UpdateConfigArgs {
    default_min_bid: Option<u64>,
    anti_snipe_window: Option<u64>,
    anti_snipe_extension: Option<u64>,
    max_extension: Option<u64>,
    max_bidders_per_auction: Option<u64>,
    max_staging_size: Option<u64>,
    max_nfts_per_bid: Option<u64>,
}

fn validate_bounded_positive(field: &str, value: u64, max: u64) -> Result<(), ContractError> {
    if value == 0 {
        return Err(ContractError::InvalidConfig {
            reason: format!("{field} must be >= 1"),
        });
    }
    if value > max {
        return Err(ContractError::InvalidConfig {
            reason: format!("{field} must be <= {max}"),
        });
    }
    Ok(())
}

/// Swap staging area: (user_addr) -> Vec<token_id>
/// Tokens sent via ReceiveNft::SwapDeposit are held here until ClaimSwap is called.
const SWAP_STAGING: cw_storage_plus::Map<&Addr, Vec<String>> =
    cw_storage_plus::Map::new("swap_staging");

// ═══════════════════════════════════════════════════════════════════════
// INSTANTIATE
// ═══════════════════════════════════════════════════════════════════════

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let admin = msg
        .admin
        .map(|a| deps.api.addr_validate(&a))
        .transpose()?
        .unwrap_or(info.sender);

    let default_min_bid = msg.default_min_bid.unwrap_or(1);
    if default_min_bid == 0 {
        return Err(ContractError::InvalidConfig {
            reason: "default_min_bid must be >= 1".to_string(),
        });
    }

    let max_bidders_per_auction = msg.max_bidders_per_auction.unwrap_or(100);
    validate_bounded_positive(
        "max_bidders_per_auction",
        max_bidders_per_auction,
        MAX_BIDDERS_PER_AUCTION_CAP,
    )?;

    let max_staging_size = msg.max_staging_size.unwrap_or(50);
    validate_bounded_positive("max_staging_size", max_staging_size, MAX_STAGING_SIZE_CAP)?;

    let max_nfts_per_bid = msg.max_nfts_per_bid.unwrap_or(50);
    validate_bounded_positive("max_nfts_per_bid", max_nfts_per_bid, MAX_NFTS_PER_BID_CAP)?;

    let mad_collection = deps.api.addr_validate(&msg.mad_scientist_collection)?;
    let mega_collection = deps.api.addr_validate(&msg.mega_mad_scientist_collection)?;

    // Prevent routing ambiguity: both collections must be distinct contracts
    if mad_collection == mega_collection {
        return Err(ContractError::CollectionAddressCollision {});
    }

    let config = Config {
        admin,
        pending_admin: None,
        paused: false,
        mad_scientist_collection: mad_collection,
        mega_mad_scientist_collection: mega_collection,
        default_min_bid,
        anti_snipe_window: msg.anti_snipe_window.unwrap_or(300),
        anti_snipe_extension: msg.anti_snipe_extension.unwrap_or(300),
        max_extension: msg.max_extension.unwrap_or(86400), // 24 hours default cap
        max_bidders_per_auction,
        max_staging_size,
        max_nfts_per_bid,
    };

    CONFIG.save(deps.storage, &config)?;
    NEXT_AUCTION_ID.save(deps.storage, &1u64)?;
    POOL_SIZE.save(deps.storage, &0u64)?;

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("admin", config.admin.to_string())
        .add_attribute(
            "mad_scientist_collection",
            config.mad_scientist_collection.to_string(),
        )
        .add_attribute(
            "mega_mad_scientist_collection",
            config.mega_mad_scientist_collection.to_string(),
        ))
}

// ═══════════════════════════════════════════════════════════════════════
// EXECUTE
// ═══════════════════════════════════════════════════════════════════════

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    // Reject any native funds sent with messages — this contract is NFT-only.
    // Without this check, accidentally attached tokens become permanently stuck.
    if !info.funds.is_empty() {
        return Err(ContractError::FundsNotAllowed {});
    }

    match msg {
        ExecuteMsg::FinalizeAuction { auction_id } => {
            execute_finalize_auction(deps, env, info, auction_id)
        }
        ExecuteMsg::CancelAuction { auction_id } => {
            execute_cancel_auction(deps, env, info, auction_id)
        }
        ExecuteMsg::WithdrawBid { auction_id } => execute_withdraw_bid(deps, env, info, auction_id),
        ExecuteMsg::ClaimSwap { requested_ids } => {
            execute_claim_swap(deps, env, info, requested_ids)
        }
        ExecuteMsg::WithdrawStaged {} => execute_withdraw_staged(deps, info),
        ExecuteMsg::SetPaused { paused } => execute_set_paused(deps, info, paused),
        ExecuteMsg::ProposeAdmin { new_admin } => execute_propose_admin(deps, info, new_admin),
        ExecuteMsg::AcceptAdmin {} => execute_accept_admin(deps, info),
        ExecuteMsg::ForceCompleteAuction { auction_id } => {
            execute_force_complete(deps, info, auction_id)
        }
        ExecuteMsg::UpdateConfig {
            default_min_bid,
            anti_snipe_window,
            anti_snipe_extension,
            max_extension,
            max_bidders_per_auction,
            max_staging_size,
            max_nfts_per_bid,
        } => execute_update_config(
            deps,
            info,
            UpdateConfigArgs {
                default_min_bid,
                anti_snipe_window,
                anti_snipe_extension,
                max_extension,
                max_bidders_per_auction,
                max_staging_size,
                max_nfts_per_bid,
            },
        ),
        ExecuteMsg::ReceiveNft(receive_msg) => execute_receive_nft(deps, env, info, receive_msg),
    }
}

// ── CW721 Receive Hook ───────────────────────────────────────────────
// FIX #1/#2/#3: ALL NFT deposits now flow exclusively through ReceiveNft.
// No more direct PlaceBid or TransferNft calls from the contract.

fn execute_receive_nft(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    receive_msg: Cw721ReceiveMsg,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    // HARDENING: Pause check — blocks all incoming NFT operations
    if config.paused {
        return Err(ContractError::ContractPaused {});
    }

    let sender_collection = info.sender.clone();
    let nft_sender = deps.api.addr_validate(&receive_msg.sender)?;
    let token_id = receive_msg.token_id;

    // Decode the inner action message
    let action: ReceiveNftAction = cosmwasm_std::from_json(&receive_msg.msg)
        .map_err(|_| ContractError::InvalidReceiveMsg {})?;

    match action {
        ReceiveNftAction::Bid { auction_id } => {
            // Must come from the Standard Mad Scientist collection
            if sender_collection != config.mad_scientist_collection {
                return Err(ContractError::UnexpectedCollection {
                    collection: sender_collection.to_string(),
                });
            }
            // NFT is already transferred to this contract by the CW721 Send mechanism.
            // Just update escrow and bid state — no transfer messages needed.
            execute_receive_bid(deps, env, nft_sender, auction_id, token_id)
        }
        ReceiveNftAction::DepositMega {
            start_time,
            end_time,
            min_bid,
        } => {
            // Must come from the Cosmic Mad Scientist collection
            if sender_collection != config.mega_mad_scientist_collection {
                return Err(ContractError::UnexpectedCollection {
                    collection: sender_collection.to_string(),
                });
            }
            // Only admin can deposit Cosmic NFTs for auction
            if nft_sender != config.admin {
                return Err(ContractError::Unauthorized {});
            }
            // NFT is already in the contract. Create the auction.
            execute_create_auction(
                deps, env, nft_sender, token_id, start_time, end_time, min_bid,
            )
        }
        ReceiveNftAction::SwapDeposit => {
            // Must come from the Standard Mad Scientist collection
            if sender_collection != config.mad_scientist_collection {
                return Err(ContractError::UnexpectedCollection {
                    collection: sender_collection.to_string(),
                });
            }
            // FIX #3: Stage the token for a future ClaimSwap call.
            // NFT is already in the contract.
            execute_swap_deposit(deps, nft_sender, token_id)
        }
    }
}

// ── Create Auction (via ReceiveNft::DepositMega only) ─────────────────

fn execute_create_auction(
    deps: DepsMut,
    _env: Env,
    admin: Addr,
    mega_token_id: String,
    start_time: u64,
    end_time: u64,
    min_bid: Option<u64>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    // Validate times
    if start_time >= end_time {
        return Err(ContractError::InvalidAuctionTimes {
            start_time,
            end_time,
        });
    }

    // FIX #6: Check no active auction exists for this Cosmic token
    if let Some(existing_id) = ACTIVE_MEGA_AUCTIONS.may_load(deps.storage, &mega_token_id)? {
        return Err(ContractError::DuplicateMegaAuction {
            token_id: mega_token_id,
            auction_id: existing_id,
        });
    }

    let auction_id = NEXT_AUCTION_ID.load(deps.storage)?;
    let min = min_bid.unwrap_or(config.default_min_bid);
    if min == 0 {
        return Err(ContractError::InvalidConfig {
            reason: "auction min_bid must be >= 1".to_string(),
        });
    }

    let auction = Auction {
        auction_id,
        depositor: admin.clone(),
        mega_token_id: mega_token_id.clone(),
        start_time,
        end_time,
        original_end_time: end_time,
        status: AuctionStatus::Active,
        min_bid: min,
        highest_bid_count: 0,
        highest_bidder: None,
        highest_bid_time: 0,
        total_bidders: 0, // HARDENING: tracks unique bidders, not just highest
    };

    AUCTIONS.save(deps.storage, auction_id, &auction)?;
    NEXT_AUCTION_ID.save(deps.storage, &(auction_id + 1))?;
    ACTIVE_MEGA_AUCTIONS.save(deps.storage, &mega_token_id, &auction_id)?;

    Ok(Response::new()
        .add_attribute("action", "create_auction")
        .add_attribute("auction_id", auction_id.to_string())
        .add_attribute("mega_token_id", mega_token_id)
        .add_attribute("depositor", admin.to_string())
        .add_attribute("start_time", start_time.to_string())
        .add_attribute("end_time", end_time.to_string())
        .add_attribute("min_bid", min.to_string()))
}

// ── Receive Bid (via ReceiveNft::Bid only) ────────────────────────────
// FIX #1/#2: This is the ONLY way to bid. NFTs arrive via CW721 Send,
// so they are already in the contract. No TransferNft messages emitted.

fn execute_receive_bid(
    deps: DepsMut,
    env: Env,
    bidder: Addr,
    auction_id: u64,
    token_id: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut auction = AUCTIONS
        .may_load(deps.storage, auction_id)?
        .ok_or(ContractError::AuctionNotFound { auction_id })?;

    // Check auction is active
    if auction.status != AuctionStatus::Active {
        return Err(ContractError::AuctionNotActive { auction_id });
    }

    let now = env.block.time.seconds();

    // Check auction timing
    if now < auction.start_time {
        return Err(ContractError::AuctionNotStarted {
            start_time: auction.start_time,
        });
    }
    if now >= auction.end_time {
        return Err(ContractError::AuctionAlreadyEnded { auction_id });
    }

    // Prevent cross-auction "double dipping": a bid token can only be escrowed
    // in one auction at a time.
    if let Some(locked_auction_id) = ESCROWED_BID_TOKENS.may_load(deps.storage, &token_id)? {
        if locked_auction_id != auction_id {
            return Err(ContractError::TokenAlreadyEscrowedInAuction {
                token_id,
                auction_id: locked_auction_id,
            });
        }
        return Err(ContractError::DuplicateTokenId { token_id });
    }

    // Load existing escrow for cumulative bidding
    let mut escrowed: Vec<String> = ESCROW
        .may_load(deps.storage, (auction_id, &bidder))?
        .unwrap_or_default();

    // HARDENING: Track new bidders and enforce max bidders cap
    let is_new_bidder = escrowed.is_empty();
    if is_new_bidder {
        if auction.total_bidders >= config.max_bidders_per_auction {
            return Err(ContractError::MaxBiddersReached {
                auction_id,
                max: config.max_bidders_per_auction,
            });
        }
        // HARDENING: Increment total_bidders on first escrow deposit
        auction.total_bidders += 1;
    }

    // HARDENING: Cap per-bidder escrow size to prevent storage bloat
    if escrowed.len() as u64 >= config.max_nfts_per_bid {
        return Err(ContractError::MaxEscrowPerBidder {
            auction_id,
            max: config.max_nfts_per_bid,
        });
    }

    // Check this token isn't already escrowed by this bidder
    if escrowed.contains(&token_id) {
        return Err(ContractError::DuplicateTokenId { token_id });
    }

    escrowed.push(token_id.clone());
    let total_bid_count = escrowed.len() as u64;

    // Always accept the NFT into escrow first — min_bid is only checked
    // when determining if this bidder becomes the highest. This allows
    // cumulative bidding (one NFT at a time) even when min_bid > 1.

    // Determine if this bid qualifies as highest:
    // 1. Must meet minimum bid threshold
    // 2. Must strictly exceed current highest (tie-breaking: earlier bidder keeps lead)
    let meets_minimum = total_bid_count >= auction.min_bid;
    let is_same_bidder = auction.highest_bidder.as_ref() == Some(&bidder);
    let exceeds_highest =
        total_bid_count > auction.highest_bid_count || auction.highest_bidder.is_none();
    let qualifies_as_highest = meets_minimum && (exceeds_highest || is_same_bidder);

    if !qualifies_as_highest {
        // Accept into escrow but don't update highest bidder
        ESCROW.save(deps.storage, (auction_id, &bidder), &escrowed)?;
        ESCROWED_BID_TOKENS.save(deps.storage, &token_id, &auction_id)?;
        let bid = Bid {
            bidder: bidder.clone(),
            token_ids: escrowed,
            timestamp: now,
        };
        BIDS.save(deps.storage, (auction_id, &bidder), &bid)?;
        // Save auction (total_bidders may have changed)
        AUCTIONS.save(deps.storage, auction_id, &auction)?;

        return Ok(Response::new()
            .add_attribute("action", "place_bid")
            .add_attribute("auction_id", auction_id.to_string())
            .add_attribute("bidder", bidder.to_string())
            .add_attribute("token_id", token_id)
            .add_attribute("total_bid_count", total_bid_count.to_string())
            .add_attribute("is_highest", "false")
            .add_attribute("new_end_time", auction.end_time.to_string()));
    }

    // Update auction highest bid info
    auction.highest_bid_count = total_bid_count;
    auction.highest_bidder = Some(bidder.clone());
    auction.highest_bid_time = now;

    // FIX #5: Anti-sniping with max extension cap
    if now.saturating_add(config.anti_snipe_window) >= auction.end_time && now <= auction.end_time {
        let max_allowed = auction
            .original_end_time
            .saturating_add(config.max_extension);
        let proposed = now.saturating_add(config.anti_snipe_extension);
        auction.end_time = proposed.min(max_allowed);
    }

    // Save bid and escrow
    let bid = Bid {
        bidder: bidder.clone(),
        token_ids: escrowed.clone(),
        timestamp: now,
    };
    BIDS.save(deps.storage, (auction_id, &bidder), &bid)?;
    ESCROW.save(deps.storage, (auction_id, &bidder), &escrowed)?;
    ESCROWED_BID_TOKENS.save(deps.storage, &token_id, &auction_id)?;
    AUCTIONS.save(deps.storage, auction_id, &auction)?;

    Ok(Response::new()
        .add_attribute("action", "place_bid")
        .add_attribute("auction_id", auction_id.to_string())
        .add_attribute("bidder", bidder.to_string())
        .add_attribute("token_id", token_id)
        .add_attribute("total_bid_count", total_bid_count.to_string())
        .add_attribute("is_highest", "true")
        .add_attribute("new_end_time", auction.end_time.to_string()))
}

// ── Finalize Auction ──────────────────────────────────────────────────
// FIX #4: Only transfers the Cosmic NFT to winner and deposits winner's NFTs
// into pool. Losers must call WithdrawBid themselves (self-claim pattern).

fn execute_finalize_auction(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    auction_id: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut auction = AUCTIONS
        .may_load(deps.storage, auction_id)?
        .ok_or(ContractError::AuctionNotFound { auction_id })?;

    if auction.status != AuctionStatus::Active {
        return Err(ContractError::AuctionNotActive { auction_id });
    }

    let now = env.block.time.seconds();
    if now < auction.end_time {
        return Err(ContractError::AuctionNotEnded {
            auction_id,
            end_time: auction.end_time,
        });
    }

    let mut msgs: Vec<CosmosMsg> = vec![];

    if let Some(ref winner) = auction.highest_bidder {
        // Transfer Cosmic Mad Scientist to winner
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.mega_mad_scientist_collection.to_string(),
            msg: to_json_binary(&Cw721ExecEmpty::TransferNft {
                recipient: winner.to_string(),
                token_id: auction.mega_token_id.clone(),
            })?,
            funds: vec![],
        }));

        // Winner's NFTs go to the swap pool
        if let Some(winner_tokens) = ESCROW.may_load(deps.storage, (auction_id, winner))? {
            let mut pool_size = POOL_SIZE.load(deps.storage)?;
            for tid in &winner_tokens {
                POOL.save(deps.storage, tid, &())?;
                ESCROWED_BID_TOKENS.remove(deps.storage, tid);
                pool_size += 1;
            }
            POOL_SIZE.save(deps.storage, &pool_size)?;
            ESCROW.remove(deps.storage, (auction_id, winner));
            BIDS.remove(deps.storage, (auction_id, winner));
        }

        // Mark as Finalizing — losers must call WithdrawBid
        auction.status = AuctionStatus::Finalizing;
    } else {
        // No qualifying bids — return the Cosmic NFT to original depositor.
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.mega_mad_scientist_collection.to_string(),
            msg: to_json_binary(&Cw721ExecEmpty::TransferNft {
                recipient: auction.depositor.to_string(),
                token_id: auction.mega_token_id.clone(),
            })?,
            funds: vec![],
        }));
        auction.status = AuctionStatus::Completed;
    }

    // Clear the active cosmic auction guard
    ACTIVE_MEGA_AUCTIONS.remove(deps.storage, &auction.mega_token_id);
    AUCTIONS.save(deps.storage, auction_id, &auction)?;

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "finalize_auction")
        .add_attribute("auction_id", auction_id.to_string())
        .add_attribute(
            "winner",
            auction
                .highest_bidder
                .map(|a| a.to_string())
                .unwrap_or_else(|| "none".to_string()),
        )
        .add_attribute("winning_bid_count", auction.highest_bid_count.to_string())
        .add_attribute("status", format!("{:?}", auction.status)))
}

// ── Withdraw Bid ──────────────────────────────────────────────────────
// FIX #4: Losers self-claim their NFTs. This avoids gas limit issues
// from returning all losers' NFTs in a single transaction.

fn execute_withdraw_bid(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    auction_id: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let auction = AUCTIONS
        .may_load(deps.storage, auction_id)?
        .ok_or(ContractError::AuctionNotFound { auction_id })?;

    // Allow withdrawal after finalization/cancellation. Completed is allowed
    // to support force-complete and "no qualifying winner" finalizations.
    match auction.status {
        AuctionStatus::Finalizing | AuctionStatus::Cancelled | AuctionStatus::Completed => {}
        _ => {
            return Err(ContractError::WithdrawNotAllowed { auction_id });
        }
    }

    // Load this bidder's escrowed NFTs
    let escrowed = ESCROW
        .may_load(deps.storage, (auction_id, &info.sender))?
        .ok_or(ContractError::NothingToWithdraw { auction_id })?;

    if escrowed.is_empty() {
        return Err(ContractError::NothingToWithdraw { auction_id });
    }

    // Build transfer messages to return NFTs to the bidder
    let mut msgs: Vec<CosmosMsg> = vec![];
    for tid in &escrowed {
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.mad_scientist_collection.to_string(),
            msg: to_json_binary(&Cw721ExecEmpty::TransferNft {
                recipient: info.sender.to_string(),
                token_id: tid.clone(),
            })?,
            funds: vec![],
        }));
    }

    for tid in &escrowed {
        ESCROWED_BID_TOKENS.remove(deps.storage, tid);
    }

    // Clean up storage (FIX #9)
    ESCROW.remove(deps.storage, (auction_id, &info.sender));
    BIDS.remove(deps.storage, (auction_id, &info.sender));

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "withdraw_bid")
        .add_attribute("auction_id", auction_id.to_string())
        .add_attribute("bidder", info.sender.to_string())
        .add_attribute("nfts_returned", escrowed.len().to_string()))
}

// ── Cancel Auction ────────────────────────────────────────────────────
// FIX #7: Only allowed before any bids are placed.
// HARDENING: Now checks total_bidders instead of total_bids to prevent
// cancellation when NFTs are escrowed (even sub-minimum bids).

fn execute_cancel_auction(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    auction_id: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.admin {
        return Err(ContractError::Unauthorized {});
    }

    let mut auction = AUCTIONS
        .may_load(deps.storage, auction_id)?
        .ok_or(ContractError::AuctionNotFound { auction_id })?;

    if auction.status != AuctionStatus::Active {
        return Err(ContractError::AuctionNotActive { auction_id });
    }

    // HARDENING: Check total_bidders (any escrow deposit) instead of old total_bids
    if auction.total_bidders > 0 {
        return Err(ContractError::CannotCancelWithBids { auction_id });
    }

    // Return the Cosmic NFT to original depositor
    let msgs: Vec<CosmosMsg> = vec![CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.mega_mad_scientist_collection.to_string(),
        msg: to_json_binary(&Cw721ExecEmpty::TransferNft {
            recipient: auction.depositor.to_string(),
            token_id: auction.mega_token_id.clone(),
        })?,
        funds: vec![],
    })];

    // Clear active cosmic auction guard
    ACTIVE_MEGA_AUCTIONS.remove(deps.storage, &auction.mega_token_id);

    auction.status = AuctionStatus::Cancelled;
    AUCTIONS.save(deps.storage, auction_id, &auction)?;

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "cancel_auction")
        .add_attribute("auction_id", auction_id.to_string()))
}

// ── Swap Deposit (via ReceiveNft::SwapDeposit) ────────────────────────
// FIX #3: Users send NFTs to the contract via CW721 Send with SwapDeposit
// action. Tokens are staged until ClaimSwap is called.

fn execute_swap_deposit(
    deps: DepsMut,
    sender: Addr,
    token_id: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut staged: Vec<String> = SWAP_STAGING
        .may_load(deps.storage, &sender)?
        .unwrap_or_default();

    // HARDENING: Cap staging size
    if staged.len() as u64 >= config.max_staging_size {
        return Err(ContractError::StagingLimitReached {
            max: config.max_staging_size,
        });
    }

    if staged.contains(&token_id) {
        return Err(ContractError::DuplicateTokenId { token_id });
    }

    staged.push(token_id.clone());
    SWAP_STAGING.save(deps.storage, &sender, &staged)?;

    Ok(Response::new()
        .add_attribute("action", "swap_deposit")
        .add_attribute("sender", sender.to_string())
        .add_attribute("token_id", token_id)
        .add_attribute("total_staged", staged.len().to_string()))
}

// ── Claim Swap ────────────────────────────────────────────────────────
// FIX #3: After depositing offered tokens via SwapDeposit, call ClaimSwap
// to specify which pool tokens you want. 1:1 ratio enforced.

fn execute_claim_swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    requested_ids: Vec<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    // Load staged tokens
    let staged = SWAP_STAGING
        .may_load(deps.storage, &info.sender)?
        .ok_or(ContractError::NoStagedTokens {})?;

    if staged.is_empty() {
        return Err(ContractError::NoStagedTokens {});
    }

    if requested_ids.is_empty() {
        return Err(ContractError::SwapEmpty {});
    }

    // 1:1 ratio
    if staged.len() != requested_ids.len() {
        return Err(ContractError::SwapCountMismatch {
            staged: staged.len() as u64,
            requested: requested_ids.len() as u64,
        });
    }

    // Check for duplicates in requested
    let mut seen_requested = std::collections::HashSet::new();
    for tid in &requested_ids {
        if !seen_requested.insert(tid.clone()) {
            return Err(ContractError::DuplicateTokenId {
                token_id: tid.clone(),
            });
        }
    }

    // Check no overlap between staged and requested
    let staged_set: std::collections::HashSet<String> = staged.iter().cloned().collect();
    for tid in &requested_ids {
        if staged_set.contains(tid) {
            return Err(ContractError::SwapOverlap {
                token_id: tid.clone(),
            });
        }
    }

    // Verify all requested tokens are in the pool
    for tid in &requested_ids {
        if !POOL.has(deps.storage, tid) {
            return Err(ContractError::TokenNotInPool {
                token_id: tid.clone(),
            });
        }
    }

    // Execute the swap:
    // 1. Move staged tokens into the pool
    // 2. Remove requested tokens from the pool and transfer to caller
    // Pool size stays the same (N in, N out)

    for tid in &staged {
        POOL.save(deps.storage, tid, &())?;
    }

    let mut msgs: Vec<CosmosMsg> = vec![];
    for tid in &requested_ids {
        POOL.remove(deps.storage, tid);
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.mad_scientist_collection.to_string(),
            msg: to_json_binary(&Cw721ExecEmpty::TransferNft {
                recipient: info.sender.to_string(),
                token_id: tid.clone(),
            })?,
            funds: vec![],
        }));
    }

    // Clear staging area
    SWAP_STAGING.remove(deps.storage, &info.sender);

    // FIX #10: Emit swap details as attributes instead of storing on-chain
    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "claim_swap")
        .add_attribute("swapper", info.sender.to_string())
        .add_attribute("swap_count", staged.len().to_string())
        .add_attribute("offered_ids", staged.join(","))
        .add_attribute("requested_ids", requested_ids.join(","))
        .add_attribute("timestamp", env.block.time.seconds().to_string()))
}

fn execute_withdraw_staged(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let staged = SWAP_STAGING
        .may_load(deps.storage, &info.sender)?
        .ok_or(ContractError::NoStagedTokens {})?;

    if staged.is_empty() {
        return Err(ContractError::NoStagedTokens {});
    }

    let mut msgs: Vec<CosmosMsg> = vec![];
    for tid in &staged {
        msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.mad_scientist_collection.to_string(),
            msg: to_json_binary(&Cw721ExecEmpty::TransferNft {
                recipient: info.sender.to_string(),
                token_id: tid.clone(),
            })?,
            funds: vec![],
        }));
    }

    SWAP_STAGING.remove(deps.storage, &info.sender);

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "withdraw_staged")
        .add_attribute("sender", info.sender.to_string())
        .add_attribute("nfts_returned", staged.len().to_string()))
}

// ── Pause / Unpause ──────────────────────────────────────────────────
// HARDENING: Admin can freeze all incoming NFT operations if a
// vulnerability is discovered. Withdrawals remain operational.

fn execute_set_paused(
    deps: DepsMut,
    info: MessageInfo,
    paused: bool,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.admin {
        return Err(ContractError::Unauthorized {});
    }

    config.paused = paused;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "set_paused")
        .add_attribute("paused", paused.to_string()))
}

// ── Two-Step Admin Transfer ──────────────────────────────────────────
// HARDENING: Prevents accidental permanent lockout from typo'd admin address.

fn execute_propose_admin(
    deps: DepsMut,
    info: MessageInfo,
    new_admin: String,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.admin {
        return Err(ContractError::Unauthorized {});
    }

    let validated = deps.api.addr_validate(&new_admin)?;
    config.pending_admin = Some(validated.clone());
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "propose_admin")
        .add_attribute("proposed_admin", validated.to_string()))
}

fn execute_accept_admin(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    let pending = config
        .pending_admin
        .ok_or(ContractError::NoPendingAdmin {})?;
    if info.sender != pending {
        return Err(ContractError::NotPendingAdmin {});
    }

    config.admin = pending.clone();
    config.pending_admin = None;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "accept_admin")
        .add_attribute("new_admin", pending.to_string()))
}

// ── Force Complete Auction ───────────────────────────────────────────
// HARDENING: Admin can transition a Finalizing auction to Completed
// when losers refuse or are unable to withdraw, preventing permanently
// stuck state.

fn execute_force_complete(
    deps: DepsMut,
    info: MessageInfo,
    auction_id: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.admin {
        return Err(ContractError::Unauthorized {});
    }

    let mut auction = AUCTIONS
        .may_load(deps.storage, auction_id)?
        .ok_or(ContractError::AuctionNotFound { auction_id })?;

    if auction.status != AuctionStatus::Finalizing {
        return Err(ContractError::NotFinalizing { auction_id });
    }

    auction.status = AuctionStatus::Completed;
    AUCTIONS.save(deps.storage, auction_id, &auction)?;

    Ok(Response::new()
        .add_attribute("action", "force_complete_auction")
        .add_attribute("auction_id", auction_id.to_string()))
}

// ── Update Config ─────────────────────────────────────────────────────
// NOTE: Admin transfer now uses ProposeAdmin/AcceptAdmin (two-step).

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    args: UpdateConfigArgs,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.admin {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(min) = args.default_min_bid {
        if min == 0 {
            return Err(ContractError::InvalidConfig {
                reason: "default_min_bid must be >= 1".to_string(),
            });
        }
        config.default_min_bid = min;
    }
    if let Some(window) = args.anti_snipe_window {
        config.anti_snipe_window = window;
    }
    if let Some(ext) = args.anti_snipe_extension {
        config.anti_snipe_extension = ext;
    }
    if let Some(max) = args.max_extension {
        config.max_extension = max;
    }
    if let Some(max_b) = args.max_bidders_per_auction {
        validate_bounded_positive(
            "max_bidders_per_auction",
            max_b,
            MAX_BIDDERS_PER_AUCTION_CAP,
        )?;
        config.max_bidders_per_auction = max_b;
    }
    if let Some(max_s) = args.max_staging_size {
        validate_bounded_positive("max_staging_size", max_s, MAX_STAGING_SIZE_CAP)?;
        config.max_staging_size = max_s;
    }
    if let Some(max_n) = args.max_nfts_per_bid {
        validate_bounded_positive("max_nfts_per_bid", max_n, MAX_NFTS_PER_BID_CAP)?;
        config.max_nfts_per_bid = max_n;
    }

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "update_config")
        .add_attribute("admin", config.admin.to_string()))
}

// ═══════════════════════════════════════════════════════════════════════
// QUERY
// ═══════════════════════════════════════════════════════════════════════

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetConfig {} => to_json_binary(&query_config(deps)?),
        QueryMsg::GetAuction { auction_id } => to_json_binary(&query_auction(deps, auction_id)?),
        QueryMsg::GetAllAuctions {
            status,
            start_after,
            limit,
        } => to_json_binary(&query_all_auctions(deps, status, start_after, limit)?),
        QueryMsg::GetBids {
            auction_id,
            start_after,
            limit,
        } => to_json_binary(&query_bids(deps, auction_id, start_after, limit)?),
        QueryMsg::GetUserBid { auction_id, bidder } => {
            to_json_binary(&query_user_bid(deps, auction_id, bidder)?)
        }
        QueryMsg::GetPoolContents { start_after, limit } => {
            to_json_binary(&query_pool_contents(deps, start_after, limit)?)
        }
        QueryMsg::GetPoolSize {} => to_json_binary(&query_pool_size(deps)?),
        QueryMsg::GetSwapStaging { user } => to_json_binary(&query_swap_staging(deps, user)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        admin: config.admin,
        pending_admin: config.pending_admin,
        paused: config.paused,
        mad_scientist_collection: config.mad_scientist_collection,
        mega_mad_scientist_collection: config.mega_mad_scientist_collection,
        default_min_bid: config.default_min_bid,
        anti_snipe_window: config.anti_snipe_window,
        anti_snipe_extension: config.anti_snipe_extension,
        max_extension: config.max_extension,
        max_bidders_per_auction: config.max_bidders_per_auction,
        max_staging_size: config.max_staging_size,
        max_nfts_per_bid: config.max_nfts_per_bid,
    })
}

fn query_auction(deps: Deps, auction_id: u64) -> StdResult<AuctionResponse> {
    let auction = AUCTIONS.load(deps.storage, auction_id)?;

    let bids: Vec<Bid> = BIDS
        .prefix(auction_id)
        .range(deps.storage, None, None, Order::Ascending)
        .map(|r| r.map(|(_, bid)| bid))
        .collect::<StdResult<Vec<_>>>()?;

    Ok(AuctionResponse { auction, bids })
}

fn query_all_auctions(
    deps: Deps,
    status: Option<AuctionStatus>,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<AuctionsResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start_after.map(cw_storage_plus::Bound::exclusive);

    let mut auctions: Vec<Auction> = Vec::with_capacity(limit);
    for item in AUCTIONS.range(deps.storage, start, None, Order::Ascending) {
        let (_, auction) = item?;
        if let Some(ref s) = status {
            if auction.status != *s {
                continue;
            }
        }
        auctions.push(auction);
        if auctions.len() >= limit {
            break;
        }
    }

    Ok(AuctionsResponse { auctions })
}

fn query_bids(
    deps: Deps,
    auction_id: u64,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<BidsResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start_after
        .as_ref()
        .map(|s| deps.api.addr_validate(s))
        .transpose()?;
    let start_bound = start.as_ref().map(cw_storage_plus::Bound::exclusive);

    let bids: Vec<Bid> = BIDS
        .prefix(auction_id)
        .range(deps.storage, start_bound, None, Order::Ascending)
        .take(limit)
        .map(|r| r.map(|(_, bid)| bid))
        .collect::<StdResult<Vec<_>>>()?;

    Ok(BidsResponse { bids })
}

fn query_user_bid(deps: Deps, auction_id: u64, bidder: String) -> StdResult<BidResponse> {
    let bidder_addr = deps.api.addr_validate(&bidder)?;
    let bid = BIDS.may_load(deps.storage, (auction_id, &bidder_addr))?;
    Ok(BidResponse { bid })
}

fn query_pool_contents(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<PoolContentsResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start_after
        .as_deref()
        .map(cw_storage_plus::Bound::exclusive);

    let token_ids: Vec<String> = POOL
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|r| r.map(|(tid, _)| tid))
        .collect::<StdResult<Vec<_>>>()?;

    Ok(PoolContentsResponse { token_ids })
}

// FIX #8: O(1) pool size query
fn query_pool_size(deps: Deps) -> StdResult<PoolSizeResponse> {
    let size = POOL_SIZE.load(deps.storage)?;
    Ok(PoolSizeResponse { size })
}

fn query_swap_staging(deps: Deps, user: String) -> StdResult<SwapStagingResponse> {
    let user_addr = deps.api.addr_validate(&user)?;
    let token_ids = SWAP_STAGING
        .may_load(deps.storage, &user_addr)?
        .unwrap_or_default();
    Ok(SwapStagingResponse { token_ids })
}
