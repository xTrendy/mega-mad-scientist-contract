// ═══════════════════════════════════════════════════════════════════════
// MULTI-CONTRACT INTEGRATION TESTS
// ═══════════════════════════════════════════════════════════════════════
//
// Uses cw-multi-test to stand up a fake blockchain with REAL CW721
// contracts (cw721-base) and the Cosmic Mad Scientist contract all
// interacting. This tests the actual cross-contract message flow,
// not just internal logic.
//
// What this simulates:
//   1. Deploy 2 CW721 contracts (Mad + Cosmic collections)
//   2. Deploy the auction/swap contract
//   3. Mint NFTs to users
//   4. Run full auction: deposit Cosmic, bid with Standard Mad NFTs, finalize, withdraw
//   5. Run full swap: deposit to staging, claim from pool
//   6. Admin operations: pause, admin transfer, force-complete
//
// ═══════════════════════════════════════════════════════════════════════

use cosmwasm_std::{
    to_json_binary, Addr, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdError,
    StdResult, Timestamp,
};
use cw721::Cw721QueryMsg;
use cw721_base::msg::{
    ExecuteMsg as Cw721ExecuteMsg, InstantiateMsg as Cw721InstantiateMsg,
    QueryMsg as Cw721BaseQueryMsg,
};
use cw_multi_test::{App, AppBuilder, ContractWrapper, Executor};
use cw_storage_plus::Item;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::{fs, path::PathBuf};

use mega_mad_scientist::msg::{
    AuctionResponse, AuctionsResponse, ConfigResponse, ExecuteMsg, InstantiateMsg,
    PoolContentsResponse, PoolSizeResponse, QueryMsg, ReceiveNftAction, SwapStagingResponse,
};
use mega_mad_scientist::state::AuctionStatus;

// ── Addresses ────────────────────────────────────────────────────────

const ADMIN: &str = "cosmos1admin";
const BIDDER1: &str = "cosmos1bidder1";
const BIDDER2: &str = "cosmos1bidder2";
const BIDDER3: &str = "cosmos1bidder3";
const NEW_ADMIN: &str = "cosmos1newadmin";

// ── Contract wrappers ────────────────────────────────────────────────

fn auction_contract() -> Box<dyn cw_multi_test::Contract<Empty>> {
    let contract = ContractWrapper::new(
        mega_mad_scientist::contract::execute,
        mega_mad_scientist::contract::instantiate,
        mega_mad_scientist::contract::query,
    );
    Box::new(contract)
}

fn cw721_contract() -> Box<dyn cw_multi_test::Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw721_base::entry::execute,
        cw721_base::entry::instantiate,
        cw721_base::entry::query,
    );
    Box::new(contract)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct FailingCw721InstantiateMsg {
    name: String,
    symbol: String,
    minter: String,
    fail_transfer_token: Option<String>,
}

const FAIL_TRANSFER_TOKEN: Item<Option<String>> = Item::new("fail_transfer_token");

fn failing_cw721_instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: FailingCw721InstantiateMsg,
) -> Result<Response, cw721_base::ContractError> {
    FAIL_TRANSFER_TOKEN.save(deps.storage, &msg.fail_transfer_token)?;
    Ok(cw721_base::entry::instantiate(
        deps,
        env,
        info,
        Cw721InstantiateMsg {
            name: msg.name,
            symbol: msg.symbol,
            minter: msg.minter,
        },
    )?)
}

fn failing_cw721_execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: Cw721ExecuteMsg<Option<Empty>, Empty>,
) -> Result<Response, cw721_base::ContractError> {
    if let Cw721ExecuteMsg::TransferNft { token_id, .. } = &msg {
        let fail_token = FAIL_TRANSFER_TOKEN.load(deps.storage)?;
        if fail_token.as_deref() == Some(token_id.as_str()) {
            return Err(cw721_base::ContractError::Std(StdError::generic_err(
                format!("forced transfer failure for token {token_id}"),
            )));
        }
    }
    cw721_base::entry::execute(deps, env, info, msg)
}

fn failing_cw721_query(deps: Deps, env: Env, msg: Cw721BaseQueryMsg<Empty>) -> StdResult<Binary> {
    cw721_base::entry::query(deps, env, msg)
}

fn failing_cw721_contract() -> Box<dyn cw_multi_test::Contract<Empty>> {
    let contract = ContractWrapper::new(
        failing_cw721_execute,
        failing_cw721_instantiate,
        failing_cw721_query,
    );
    Box::new(contract)
}

// ── Test app builder ─────────────────────────────────────────────────

struct TestEnv {
    app: App,
    auction_addr: Addr,
    mad_addr: Addr,
    mega_addr: Addr,
}

fn setup_test_env_with_collections(
    mad_fail_transfer_token: Option<&str>,
    mega_fail_transfer_token: Option<&str>,
) -> TestEnv {
    // Build app with a starting block time
    let mut app = AppBuilder::new().build(|router, _api, storage| {
        router
            .bank
            .init_balance(
                storage,
                &Addr::unchecked(ADMIN),
                cosmwasm_std::coins(1_000_000_000, "uatom"),
            )
            .unwrap();
    });
    app.update_block(|block| {
        block.time = Timestamp::from_seconds(1000);
        block.height = 100;
    });

    // Store contract codes
    let auction_code_id = app.store_code(auction_contract());
    let cw721_code_id = app.store_code(cw721_contract());
    let failing_cw721_code_id = app.store_code(failing_cw721_contract());

    // Instantiate Mad Scientist CW721 collection
    let mad_addr = if let Some(fail_token) = mad_fail_transfer_token {
        app.instantiate_contract(
            failing_cw721_code_id,
            Addr::unchecked(ADMIN),
            &FailingCw721InstantiateMsg {
                name: "Mad Scientists".to_string(),
                symbol: "MAD".to_string(),
                minter: ADMIN.to_string(),
                fail_transfer_token: Some(fail_token.to_string()),
            },
            &[],
            "mad-scientists",
            None,
        )
        .unwrap()
    } else {
        app.instantiate_contract(
            cw721_code_id,
            Addr::unchecked(ADMIN),
            &Cw721InstantiateMsg {
                name: "Mad Scientists".to_string(),
                symbol: "MAD".to_string(),
                minter: ADMIN.to_string(),
            },
            &[],
            "mad-scientists",
            None,
        )
        .unwrap()
    };

    // Instantiate Cosmic Mad Scientist CW721 collection
    let mega_addr = if let Some(fail_token) = mega_fail_transfer_token {
        app.instantiate_contract(
            failing_cw721_code_id,
            Addr::unchecked(ADMIN),
            &FailingCw721InstantiateMsg {
                name: "Cosmic Mad Scientists".to_string(),
                symbol: "MEGA".to_string(),
                minter: ADMIN.to_string(),
                fail_transfer_token: Some(fail_token.to_string()),
            },
            &[],
            "mega-mad-scientists",
            None,
        )
        .unwrap()
    } else {
        app.instantiate_contract(
            cw721_code_id,
            Addr::unchecked(ADMIN),
            &Cw721InstantiateMsg {
                name: "Cosmic Mad Scientists".to_string(),
                symbol: "MEGA".to_string(),
                minter: ADMIN.to_string(),
            },
            &[],
            "mega-mad-scientists",
            None,
        )
        .unwrap()
    };

    // Instantiate the auction contract
    let auction_addr = app
        .instantiate_contract(
            auction_code_id,
            Addr::unchecked(ADMIN),
            &InstantiateMsg {
                admin: Some(ADMIN.to_string()),
                mad_scientist_collection: mad_addr.to_string(),
                mega_mad_scientist_collection: mega_addr.to_string(),
                default_min_bid: Some(2),
                anti_snipe_window: Some(300),
                anti_snipe_extension: Some(300),
                max_extension: Some(86400),
                max_bidders_per_auction: Some(100),
                max_staging_size: Some(50),
                max_nfts_per_bid: Some(50),
            },
            &[],
            "mega-mad-scientist-auction",
            None,
        )
        .unwrap();

    TestEnv {
        app,
        auction_addr,
        mad_addr,
        mega_addr,
    }
}

fn setup_test_env() -> TestEnv {
    setup_test_env_with_collections(None, None)
}

// ── Helpers ──────────────────────────────────────────────────────────

fn mint_mad(env: &mut TestEnv, to: &str, token_id: &str) {
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.mad_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::Mint {
                token_id: token_id.to_string(),
                owner: to.to_string(),
                token_uri: None,
                extension: Empty {},
            },
            &[],
        )
        .unwrap();
}

fn mint_mega(env: &mut TestEnv, to: &str, token_id: &str) {
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.mega_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::Mint {
                token_id: token_id.to_string(),
                owner: to.to_string(),
                token_uri: None,
                extension: Empty {},
            },
            &[],
        )
        .unwrap();
}

/// Send a Cosmic NFT to the auction contract to create an auction
fn send_mega_for_auction(
    env: &mut TestEnv,
    from: &str,
    token_id: &str,
    start_time: u64,
    end_time: u64,
    min_bid: Option<u64>,
) {
    env.app
        .execute_contract(
            Addr::unchecked(from),
            env.mega_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::SendNft {
                contract: env.auction_addr.to_string(),
                token_id: token_id.to_string(),
                msg: to_json_binary(&ReceiveNftAction::DepositMega {
                    start_time,
                    end_time,
                    min_bid,
                })
                .unwrap(),
            },
            &[],
        )
        .unwrap();
}

/// Send a Mad Scientist NFT to bid on an auction
fn send_bid(env: &mut TestEnv, from: &str, token_id: &str, auction_id: u64) {
    env.app
        .execute_contract(
            Addr::unchecked(from),
            env.mad_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::SendNft {
                contract: env.auction_addr.to_string(),
                token_id: token_id.to_string(),
                msg: to_json_binary(&ReceiveNftAction::Bid { auction_id }).unwrap(),
            },
            &[],
        )
        .unwrap();
}

/// Send a Mad Scientist NFT for swap staging
fn send_swap_deposit(env: &mut TestEnv, from: &str, token_id: &str) {
    env.app
        .execute_contract(
            Addr::unchecked(from),
            env.mad_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::SendNft {
                contract: env.auction_addr.to_string(),
                token_id: token_id.to_string(),
                msg: to_json_binary(&ReceiveNftAction::SwapDeposit).unwrap(),
            },
            &[],
        )
        .unwrap();
}

fn advance_time(env: &mut TestEnv, seconds: u64) {
    env.app.update_block(|block| {
        block.time = block.time.plus_seconds(seconds);
        block.height += seconds / 5; // ~5s per block
    });
}

fn set_time(env: &mut TestEnv, seconds: u64) {
    env.app.update_block(|block| {
        block.time = Timestamp::from_seconds(seconds);
    });
}

fn query_config(env: &TestEnv) -> ConfigResponse {
    env.app
        .wrap()
        .query_wasm_smart(&env.auction_addr, &QueryMsg::GetConfig {})
        .unwrap()
}

fn query_auction(env: &TestEnv, auction_id: u64) -> AuctionResponse {
    env.app
        .wrap()
        .query_wasm_smart(&env.auction_addr, &QueryMsg::GetAuction { auction_id })
        .unwrap()
}

fn query_pool_size(env: &TestEnv) -> u64 {
    let resp: PoolSizeResponse = env
        .app
        .wrap()
        .query_wasm_smart(&env.auction_addr, &QueryMsg::GetPoolSize {})
        .unwrap();
    resp.size
}

fn query_all_auctions(env: &TestEnv) -> AuctionsResponse {
    env.app
        .wrap()
        .query_wasm_smart(
            &env.auction_addr,
            &QueryMsg::GetAllAuctions {
                status: None,
                start_after: None,
                limit: None,
            },
        )
        .unwrap()
}

fn query_pool_contents(env: &TestEnv) -> Vec<String> {
    let resp: PoolContentsResponse = env
        .app
        .wrap()
        .query_wasm_smart(
            &env.auction_addr,
            &QueryMsg::GetPoolContents {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    resp.token_ids
}

fn query_staging(env: &TestEnv, user: &str) -> Vec<String> {
    let resp: SwapStagingResponse = env
        .app
        .wrap()
        .query_wasm_smart(
            &env.auction_addr,
            &QueryMsg::GetSwapStaging {
                user: user.to_string(),
            },
        )
        .unwrap();
    resp.token_ids
}

/// Query who owns a specific NFT on a CW721 contract
fn query_nft_owner(env: &TestEnv, collection: &Addr, token_id: &str) -> String {
    let resp: cw721::OwnerOfResponse = env
        .app
        .wrap()
        .query_wasm_smart(
            collection,
            &Cw721QueryMsg::OwnerOf {
                token_id: token_id.to_string(),
                include_expired: None,
            },
        )
        .unwrap();
    resp.owner
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FULL AUCTION LIFECYCLE
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_full_auction_lifecycle() {
    let mut env = setup_test_env();

    // ── 1. Mint NFTs ──────────────────────────────────────────────────
    // Admin gets a Cosmic NFT
    mint_mega(&mut env, ADMIN, "mega_1");
    // Bidders get Standard Mad NFTs
    for i in 1..=5 {
        mint_mad(&mut env, BIDDER1, &format!("mad_{}", i));
    }
    for i in 6..=10 {
        mint_mad(&mut env, BIDDER2, &format!("mad_{}", i));
    }

    // Verify ownership
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        ADMIN
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        BIDDER1
    );

    // ── 2. Create auction ─────────────────────────────────────────────
    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));

    // Cosmic NFT should now be owned by the auction contract
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        env.auction_addr.to_string()
    );

    // Verify auction state
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Active);
    assert_eq!(auction.auction.mega_token_id, "mega_1");
    assert_eq!(auction.auction.min_bid, 2);
    assert_eq!(auction.auction.depositor.to_string(), ADMIN);

    // ── 3. Bidders place bids ─────────────────────────────────────────
    advance_time(&mut env, 1000); // now at t=2000

    // Bidder1 sends 3 NFTs (one at a time, cumulative)
    send_bid(&mut env, BIDDER1, "mad_1", 1);
    send_bid(&mut env, BIDDER1, "mad_2", 1);
    send_bid(&mut env, BIDDER1, "mad_3", 1);

    // Bidder2 sends 5 NFTs (should overtake Bidder1)
    send_bid(&mut env, BIDDER2, "mad_6", 1);
    send_bid(&mut env, BIDDER2, "mad_7", 1);
    send_bid(&mut env, BIDDER2, "mad_8", 1);
    send_bid(&mut env, BIDDER2, "mad_9", 1);
    send_bid(&mut env, BIDDER2, "mad_10", 1);

    // Verify bid state
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.highest_bid_count, 5);
    assert_eq!(
        auction.auction.highest_bidder,
        Some(Addr::unchecked(BIDDER2))
    );
    assert_eq!(auction.auction.total_bidders, 2);

    // All bid NFTs should be owned by the auction contract now
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        env.auction_addr.to_string()
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_6"),
        env.auction_addr.to_string()
    );

    // ── 4. Finalize auction ───────────────────────────────────────────
    set_time(&mut env, 6000); // past end_time

    env.app
        .execute_contract(
            Addr::unchecked(BIDDER1), // anyone can finalize
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Cosmic NFT should be transferred to the winner (BIDDER2)
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        BIDDER2
    );

    // Winner's NFTs should now be in the pool
    assert_eq!(query_pool_size(&env), 5);

    // Auction should be Finalizing
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Finalizing);

    // ── 5. Loser withdraws ────────────────────────────────────────────
    env.app
        .execute_contract(
            Addr::unchecked(BIDDER1),
            env.auction_addr.clone(),
            &ExecuteMsg::WithdrawBid { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Bidder1's NFTs should be returned
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        BIDDER1
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_2"),
        BIDDER1
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_3"),
        BIDDER1
    );

    // Pool still has 5 (winner's NFTs)
    assert_eq!(query_pool_size(&env), 5);
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FIVE SIMULTANEOUS COSMIC AUCTIONS + NO DOUBLE DIP
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_five_simultaneous_megas_no_double_dip() {
    let mut env = setup_test_env();

    // Mint 5 Cosmic NFTs and 10 Standard Mad NFTs to the same bidder.
    for i in 1..=5 {
        mint_mega(&mut env, ADMIN, &format!("mega_{}", i));
    }
    for i in 1..=10 {
        mint_mad(&mut env, BIDDER1, &format!("mad_{}", i));
    }

    // Create 5 auctions with the same time window (simultaneous).
    for i in 1..=5 {
        send_mega_for_auction(&mut env, ADMIN, &format!("mega_{}", i), 1000, 5000, Some(2));
    }

    set_time(&mut env, 2000);

    // Bid into auction 1 with mad_1.
    send_bid(&mut env, BIDDER1, "mad_1", 1);

    // Try to "double dip" mad_1 into auction 2.
    // This must fail while mad_1 is already escrowed.
    let _err = env
        .app
        .execute_contract(
            Addr::unchecked(BIDDER1),
            env.mad_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::SendNft {
                contract: env.auction_addr.to_string(),
                token_id: "mad_1".to_string(),
                msg: to_json_binary(&ReceiveNftAction::Bid { auction_id: 2 }).unwrap(),
            },
            &[],
        )
        .unwrap_err();
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        env.auction_addr.to_string()
    );
    let auction2 = query_auction(&env, 2);
    assert_eq!(auction2.auction.highest_bid_count, 0);
    assert!(auction2.auction.highest_bidder.is_none());

    // Same bidder can still win all 5 auctions with distinct tokens.
    send_bid(&mut env, BIDDER1, "mad_2", 1);
    send_bid(&mut env, BIDDER1, "mad_3", 2);
    send_bid(&mut env, BIDDER1, "mad_4", 2);
    send_bid(&mut env, BIDDER1, "mad_5", 3);
    send_bid(&mut env, BIDDER1, "mad_6", 3);
    send_bid(&mut env, BIDDER1, "mad_7", 4);
    send_bid(&mut env, BIDDER1, "mad_8", 4);
    send_bid(&mut env, BIDDER1, "mad_9", 5);
    send_bid(&mut env, BIDDER1, "mad_10", 5);

    set_time(&mut env, 6000);

    // Finalize all auctions.
    for auction_id in 1..=5 {
        env.app
            .execute_contract(
                Addr::unchecked(ADMIN),
                env.auction_addr.clone(),
                &ExecuteMsg::FinalizeAuction { auction_id },
                &[],
            )
            .unwrap();
    }

    // BIDDER1 should receive all 5 Cosmic NFTs.
    for i in 1..=5 {
        assert_eq!(
            query_nft_owner(&env, &env.mega_addr.clone(), &format!("mega_{}", i)),
            BIDDER1
        );
    }

    // All 10 winning Standard Mad NFTs should be in pool custody.
    assert_eq!(query_pool_size(&env), 10);

    // Verify each auction records BIDDER1 as the winner with 2 NFTs.
    for auction_id in 1..=5 {
        let auction = query_auction(&env, auction_id);
        assert_eq!(
            auction.auction.highest_bidder,
            Some(Addr::unchecked(BIDDER1))
        );
        assert_eq!(auction.auction.highest_bid_count, 2);
        assert_eq!(auction.auction.status, AuctionStatus::Finalizing);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FAILURE ATOMICITY (CW721 TRANSFER FAILURES)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_finalize_is_atomic_when_mega_transfer_fails() {
    let mut env = setup_test_env_with_collections(None, Some("mega_1"));

    mint_mega(&mut env, ADMIN, "mega_1");
    mint_mad(&mut env, BIDDER1, "mad_1");
    mint_mad(&mut env, BIDDER1, "mad_2");

    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));
    set_time(&mut env, 2000);
    send_bid(&mut env, BIDDER1, "mad_1", 1);
    send_bid(&mut env, BIDDER1, "mad_2", 1);

    set_time(&mut env, 6000);
    let err = env
        .app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap_err();
    assert!(err
        .root_cause()
        .to_string()
        .contains("forced transfer failure"));

    // Finalize reverted entirely: auction state and escrow are unchanged.
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Active);
    assert_eq!(
        auction.auction.highest_bidder,
        Some(Addr::unchecked(BIDDER1))
    );
    assert_eq!(auction.auction.highest_bid_count, 2);
    assert_eq!(query_pool_size(&env), 0);
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        env.auction_addr.to_string()
    );
}

#[test]
fn integration_withdraw_is_atomic_when_mad_transfer_fails() {
    let mut env = setup_test_env_with_collections(Some("mad_1"), None);

    mint_mega(&mut env, ADMIN, "mega_1");
    mint_mad(&mut env, BIDDER1, "mad_1");
    mint_mad(&mut env, BIDDER1, "mad_2");
    mint_mad(&mut env, BIDDER2, "mad_3");
    mint_mad(&mut env, BIDDER2, "mad_4");
    mint_mad(&mut env, BIDDER2, "mad_5");

    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));
    set_time(&mut env, 2000);

    send_bid(&mut env, BIDDER1, "mad_1", 1);
    send_bid(&mut env, BIDDER1, "mad_2", 1);
    send_bid(&mut env, BIDDER2, "mad_3", 1);
    send_bid(&mut env, BIDDER2, "mad_4", 1);
    send_bid(&mut env, BIDDER2, "mad_5", 1);

    set_time(&mut env, 6000);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Loser withdraw should fail on mad_1 transfer and revert fully.
    let err = env
        .app
        .execute_contract(
            Addr::unchecked(BIDDER1),
            env.auction_addr.clone(),
            &ExecuteMsg::WithdrawBid { auction_id: 1 },
            &[],
        )
        .unwrap_err();
    assert!(err
        .root_cause()
        .to_string()
        .contains("forced transfer failure"));

    // No partial withdrawal: both loser NFTs remain in contract custody.
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        env.auction_addr.to_string()
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_2"),
        env.auction_addr.to_string()
    );

    // Bid record remains, proving storage cleanup did not run partially.
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Finalizing);
    let loser_bid = auction
        .bids
        .iter()
        .find(|b| b.bidder == Addr::unchecked(BIDDER1))
        .expect("loser bid should remain after failed withdraw");
    assert_eq!(loser_bid.token_ids.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FULL SWAP FLOW
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_full_swap_flow() {
    let mut env = setup_test_env();

    // Set up a completed auction first to populate the pool
    mint_mega(&mut env, ADMIN, "mega_1");
    for i in 1..=3 {
        mint_mad(&mut env, BIDDER1, &format!("mad_{}", i));
    }
    for i in 10..=12 {
        mint_mad(&mut env, BIDDER3, &format!("mad_{}", i));
    }

    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));
    advance_time(&mut env, 1000);

    // Bidder1 bids 3 NFTs
    send_bid(&mut env, BIDDER1, "mad_1", 1);
    send_bid(&mut env, BIDDER1, "mad_2", 1);
    send_bid(&mut env, BIDDER1, "mad_3", 1);

    // Finalize — Bidder1 wins (only bidder), pool gets their 3 NFTs
    set_time(&mut env, 6000);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    assert_eq!(query_pool_size(&env), 3);

    // ── Swap: Bidder3 deposits 2 NFTs and swaps for 2 from pool ───────
    send_swap_deposit(&mut env, BIDDER3, "mad_10");
    send_swap_deposit(&mut env, BIDDER3, "mad_11");

    // Verify staging
    let staged = query_staging(&env, BIDDER3);
    assert_eq!(staged, vec!["mad_10", "mad_11"]);

    // Claim swap — ask for mad_1 and mad_2 from the pool
    env.app
        .execute_contract(
            Addr::unchecked(BIDDER3),
            env.auction_addr.clone(),
            &ExecuteMsg::ClaimSwap {
                requested_ids: vec!["mad_1".to_string(), "mad_2".to_string()],
            },
            &[],
        )
        .unwrap();

    // Bidder3 should now own mad_1 and mad_2
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        BIDDER3
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_2"),
        BIDDER3
    );

    // Pool size unchanged (2 in, 2 out, 3 total)
    assert_eq!(query_pool_size(&env), 3);

    // Staging should be cleared
    let staged = query_staging(&env, BIDDER3);
    assert!(staged.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: AUCTION WITH NO BIDS — COSMIC RETURNED TO DEPOSITOR
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_no_bids_mega_returned() {
    let mut env = setup_test_env();

    mint_mega(&mut env, ADMIN, "mega_1");
    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));

    // Nobody bids. Finalize after end.
    set_time(&mut env, 6000);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Cosmic should be returned to depositor (ADMIN)
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        ADMIN
    );

    // Auction status should be Completed (no Finalizing needed)
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Completed);

    // Pool should be empty
    assert_eq!(query_pool_size(&env), 0);
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: CANCEL AUCTION — NO BIDS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_cancel_returns_mega() {
    let mut env = setup_test_env();

    mint_mega(&mut env, ADMIN, "mega_1");
    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));

    // Cancel before any bids
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::CancelAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Cosmic returned to depositor
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        ADMIN
    );

    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Cancelled);
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: PAUSE BLOCKS NEW BIDS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_pause_blocks_deposits() {
    let mut env = setup_test_env();

    mint_mega(&mut env, ADMIN, "mega_1");
    mint_mad(&mut env, BIDDER1, "mad_1");
    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(1));
    advance_time(&mut env, 1000);

    // Pause the contract
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::SetPaused { paused: true },
            &[],
        )
        .unwrap();

    // Bidding should fail
    let err = env
        .app
        .execute_contract(
            Addr::unchecked(BIDDER1),
            env.mad_addr.clone(),
            &Cw721ExecuteMsg::<Empty, Empty>::SendNft {
                contract: env.auction_addr.to_string(),
                token_id: "mad_1".to_string(),
                msg: to_json_binary(&ReceiveNftAction::Bid { auction_id: 1 }).unwrap(),
            },
            &[],
        )
        .unwrap_err();
    // The error should contain "paused"
    assert!(err.root_cause().to_string().contains("paused"));

    // Unpause
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::SetPaused { paused: false },
            &[],
        )
        .unwrap();

    // Bidding should work again
    send_bid(&mut env, BIDDER1, "mad_1", 1);

    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.total_bidders, 1);
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: TWO-STEP ADMIN TRANSFER
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_admin_transfer() {
    let mut env = setup_test_env();

    // Propose new admin
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::ProposeAdmin {
                new_admin: NEW_ADMIN.to_string(),
            },
            &[],
        )
        .unwrap();

    let config = query_config(&env);
    assert_eq!(config.admin, Addr::unchecked(ADMIN));
    assert_eq!(config.pending_admin, Some(Addr::unchecked(NEW_ADMIN)));

    // Accept as new admin
    env.app
        .execute_contract(
            Addr::unchecked(NEW_ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::AcceptAdmin {},
            &[],
        )
        .unwrap();

    let config = query_config(&env);
    assert_eq!(config.admin, Addr::unchecked(NEW_ADMIN));
    assert!(config.pending_admin.is_none());

    // Old admin can no longer pause
    let err = env
        .app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::SetPaused { paused: true },
            &[],
        )
        .unwrap_err();
    assert!(err.root_cause().to_string().contains("Unauthorized"));
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FORCE COMPLETE STUCK AUCTION
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_force_complete() {
    let mut env = setup_test_env();

    mint_mega(&mut env, ADMIN, "mega_1");
    for i in 1..=3 {
        mint_mad(&mut env, BIDDER1, &format!("mad_{}", i));
    }
    for i in 4..=6 {
        mint_mad(&mut env, BIDDER2, &format!("mad_{}", i));
    }

    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));
    advance_time(&mut env, 1000);

    // Bidder1 bids 3, Bidder2 bids 3 (Bidder1 wins as earlier)
    send_bid(&mut env, BIDDER1, "mad_1", 1);
    send_bid(&mut env, BIDDER1, "mad_2", 1);
    send_bid(&mut env, BIDDER1, "mad_3", 1);
    send_bid(&mut env, BIDDER2, "mad_4", 1);
    send_bid(&mut env, BIDDER2, "mad_5", 1);
    send_bid(&mut env, BIDDER2, "mad_6", 1);

    // Finalize
    set_time(&mut env, 6000);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Auction is Finalizing (Bidder2 hasn't withdrawn)
    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Finalizing);

    // Admin force-completes
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::ForceCompleteAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    let auction = query_auction(&env, 1);
    assert_eq!(auction.auction.status, AuctionStatus::Completed);

    // Bidder2 can still withdraw after force-complete (Completed status now allowed)
    env.app
        .execute_contract(
            Addr::unchecked(BIDDER2),
            env.auction_addr.clone(),
            &ExecuteMsg::WithdrawBid { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Bidder2 gets their NFTs back
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_4"),
        BIDDER2
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_5"),
        BIDDER2
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_6"),
        BIDDER2
    );
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FUNDS REJECTION
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_funds_rejected() {
    let mut env = setup_test_env();

    // Try to send native tokens with a message
    let err = env
        .app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::SetPaused { paused: false },
            &[cosmwasm_std::coin(1000, "uatom")],
        )
        .unwrap_err();
    assert!(err
        .root_cause()
        .to_string()
        .contains("This contract does not accept funds"));
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: WITHDRAW STAGED — CANCEL A SWAP
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_withdraw_staged() {
    let mut env = setup_test_env();

    mint_mad(&mut env, BIDDER1, "mad_1");
    mint_mad(&mut env, BIDDER1, "mad_2");

    // Need to populate pool first (so staging makes sense)
    // Just test staging/withdraw without a pool
    send_swap_deposit(&mut env, BIDDER1, "mad_1");
    send_swap_deposit(&mut env, BIDDER1, "mad_2");

    let staged = query_staging(&env, BIDDER1);
    assert_eq!(staged.len(), 2);

    // Withdraw staged
    env.app
        .execute_contract(
            Addr::unchecked(BIDDER1),
            env.auction_addr.clone(),
            &ExecuteMsg::WithdrawStaged {},
            &[],
        )
        .unwrap();

    // NFTs returned
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_1"),
        BIDDER1
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_2"),
        BIDDER1
    );

    // Staging cleared
    let staged = query_staging(&env, BIDDER1);
    assert!(staged.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: ANTI-SNIPE EXTENSION
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_anti_snipe_extends_auction() {
    let mut env = setup_test_env();

    mint_mega(&mut env, ADMIN, "mega_1");
    mint_mad(&mut env, BIDDER1, "mad_1");
    mint_mad(&mut env, BIDDER1, "mad_2");
    mint_mad(&mut env, BIDDER2, "mad_3");
    mint_mad(&mut env, BIDDER2, "mad_4");
    mint_mad(&mut env, BIDDER2, "mad_5");

    send_mega_for_auction(&mut env, ADMIN, "mega_1", 1000, 5000, Some(2));

    // Bidder1 bids early with 2 NFTs (meets min_bid, becomes highest)
    set_time(&mut env, 2000);
    send_bid(&mut env, BIDDER1, "mad_1", 1);
    send_bid(&mut env, BIDDER1, "mad_2", 1);

    // Bidder2 snipe-bids at 4800 (within 300s anti-snipe window of end 5000)
    set_time(&mut env, 4800);
    send_bid(&mut env, BIDDER2, "mad_3", 1);
    send_bid(&mut env, BIDDER2, "mad_4", 1);
    send_bid(&mut env, BIDDER2, "mad_5", 1); // 3 > 2, overtakes

    // Auction end_time should be extended
    let auction = query_auction(&env, 1);
    assert!(auction.auction.end_time > 5000);
    // Should be 4800 + 300 = 5100
    assert_eq!(auction.auction.end_time, 5100);

    // Cannot finalize before new end_time
    set_time(&mut env, 5050);
    let err = env
        .app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap_err();
    assert!(err.root_cause().to_string().contains("has not ended"));

    // CAN finalize after extended end
    set_time(&mut env, 5200);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Bidder2 wins
    assert_eq!(
        query_nft_owner(&env, &env.mega_addr.clone(), "mega_1"),
        BIDDER2
    );
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: STRESS MANY BIDDERS + LOSER WITHDRAW FANOUT
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_stress_many_bidders_and_withdraw_fanout() {
    let mut env = setup_test_env();

    mint_mega(&mut env, ADMIN, "mega_stress_1");
    send_mega_for_auction(&mut env, ADMIN, "mega_stress_1", 1000, 5000, Some(1));
    set_time(&mut env, 2000);

    // 80 distinct bidders each submit one NFT.
    let bidder_count = 80_u32;
    for i in 0..bidder_count {
        let bidder = format!("stress_bidder_{i:03}");
        let token = format!("mad_stress_bid_{i:03}");
        mint_mad(&mut env, &bidder, &token);
        send_bid(&mut env, &bidder, &token, 1);
    }

    set_time(&mut env, 6000);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    // Tie behavior: first bidder remains leader at bid_count=1.
    let auction = query_auction(&env, 1);
    assert_eq!(
        auction.auction.highest_bidder,
        Some(Addr::unchecked("stress_bidder_000"))
    );
    assert_eq!(auction.auction.highest_bid_count, 1);
    assert_eq!(query_pool_size(&env), 1);

    // Stress loser-withdraw fanout path across many users.
    for i in 1..bidder_count {
        let bidder = format!("stress_bidder_{i:03}");
        let token = format!("mad_stress_bid_{i:03}");
        env.app
            .execute_contract(
                Addr::unchecked(bidder.clone()),
                env.auction_addr.clone(),
                &ExecuteMsg::WithdrawBid { auction_id: 1 },
                &[],
            )
            .unwrap();
        assert_eq!(query_nft_owner(&env, &env.mad_addr.clone(), &token), bidder,);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: STRESS LARGE ESCROW + LARGE SWAP CYCLE
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn integration_stress_large_escrow_and_swap_cycle() {
    let mut env = setup_test_env();

    // Raise caps so we can stress large single-user escrow and staging paths.
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::UpdateConfig {
                default_min_bid: None,
                anti_snipe_window: None,
                anti_snipe_extension: None,
                max_extension: None,
                max_bidders_per_auction: None,
                max_staging_size: Some(150),
                max_nfts_per_bid: Some(150),
            },
            &[],
        )
        .unwrap();

    mint_mega(&mut env, ADMIN, "mega_stress_2");
    send_mega_for_auction(&mut env, ADMIN, "mega_stress_2", 1000, 5000, Some(1));
    set_time(&mut env, 2000);

    // Bidder1 loads 120 NFTs into one auction.
    for i in 0..120_u32 {
        let token = format!("mad_bulk_bid_{i:03}");
        mint_mad(&mut env, BIDDER1, &token);
        send_bid(&mut env, BIDDER1, &token, 1);
    }

    set_time(&mut env, 6000);
    env.app
        .execute_contract(
            Addr::unchecked(ADMIN),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id: 1 },
            &[],
        )
        .unwrap();

    assert_eq!(query_pool_size(&env), 120);

    // Bidder2 stages 120 NFTs to claim 120 from pool in one swap.
    for i in 0..120_u32 {
        let token = format!("mad_bulk_swap_{i:03}");
        mint_mad(&mut env, BIDDER2, &token);
        send_swap_deposit(&mut env, BIDDER2, &token);
    }

    let requested_ids: Vec<String> = (0..120_u32)
        .map(|i| format!("mad_bulk_bid_{i:03}"))
        .collect();
    env.app
        .execute_contract(
            Addr::unchecked(BIDDER2),
            env.auction_addr.clone(),
            &ExecuteMsg::ClaimSwap { requested_ids },
            &[],
        )
        .unwrap();

    // Pool size remains stable after 120-in / 120-out.
    assert_eq!(query_pool_size(&env), 120);
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_bulk_bid_000"),
        BIDDER2
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_bulk_bid_060"),
        BIDDER2
    );
    assert_eq!(
        query_nft_owner(&env, &env.mad_addr.clone(), "mad_bulk_bid_119"),
        BIDDER2
    );
    assert!(query_staging(&env, BIDDER2).is_empty());
}

// ═══════════════════════════════════════════════════════════════════════
// DETERMINISM REPLAY HARNESS (P0)
// ═══════════════════════════════════════════════════════════════════════

struct ReplayResult {
    trace: Vec<String>,
    snapshot: String,
    hash: u64,
}

fn lcg_next(x: &mut u64) -> u64 {
    // Numerical Recipes LCG constants. Deterministic across runs/platforms.
    *x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
    *x
}

fn stable_fnv1a_64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn canonical_state_snapshot(
    env: &TestEnv,
    cosmic_token_ids: &[String],
    tracked_mad_tokens: &[String],
) -> String {
    let mut out = String::new();

    let config = query_config(env);
    writeln!(&mut out, "CONFIG:{config:?}").unwrap();

    let mut auctions = query_all_auctions(env).auctions;
    auctions.sort_by_key(|a| a.auction_id);
    writeln!(&mut out, "AUCTION_COUNT:{}", auctions.len()).unwrap();
    for auction in auctions {
        let mut resp = query_auction(env, auction.auction_id);
        for bid in &mut resp.bids {
            bid.token_ids.sort();
        }
        resp.bids.sort_by(|a, b| a.bidder.cmp(&b.bidder));
        writeln!(&mut out, "AUCTION:{}:{resp:?}", auction.auction_id).unwrap();
    }

    let mut pool = query_pool_contents(env);
    pool.sort();
    writeln!(&mut out, "POOL_SIZE:{}", query_pool_size(env)).unwrap();
    writeln!(&mut out, "POOL:{pool:?}").unwrap();

    for user in [BIDDER1, BIDDER2, BIDDER3] {
        let mut staged = query_staging(env, user);
        staged.sort();
        writeln!(&mut out, "STAGING:{user}:{staged:?}").unwrap();
    }

    let mut cosmic = cosmic_token_ids.to_vec();
    cosmic.sort();
    for token_id in cosmic {
        let owner = query_nft_owner(env, &env.mega_addr, &token_id);
        writeln!(&mut out, "COSMIC_OWNER:{token_id}:{owner}").unwrap();
    }

    let mut mad = tracked_mad_tokens.to_vec();
    mad.sort();
    for token_id in mad {
        let owner = query_nft_owner(env, &env.mad_addr, &token_id);
        writeln!(&mut out, "MAD_OWNER:{token_id}:{owner}").unwrap();
    }

    out
}

fn run_deterministic_replay(seed: u64) -> ReplayResult {
    let mut rng = seed;
    let mut env = setup_test_env();
    let mut trace = Vec::new();

    let cosmic_token_ids: Vec<String> = (1..=3).map(|i| format!("mega_replay_{i}")).collect();
    for token_id in &cosmic_token_ids {
        mint_mega(&mut env, ADMIN, token_id);
        send_mega_for_auction(&mut env, ADMIN, token_id, 1000, 5000, Some(2));
        trace.push(format!("create_auction:{token_id}:ok"));
    }

    let bidders = [BIDDER1, BIDDER2, BIDDER3];
    let mut tracked_mad_tokens = Vec::new();

    for i in 0..12_u32 {
        for bidder in bidders {
            let token_id = format!("mad_replay_{}_{}", bidder.trim_start_matches("cosmos1"), i);
            mint_mad(&mut env, bidder, &token_id);
            tracked_mad_tokens.push(token_id.clone());

            let auction_id = (lcg_next(&mut rng) % 3) + 1;
            send_bid(&mut env, bidder, &token_id, auction_id);
            trace.push(format!("bid:{bidder}:{token_id}:{auction_id}:ok"));
        }
    }

    set_time(&mut env, 6000);
    for auction_id in 1..=3_u64 {
        let caller = bidders[(lcg_next(&mut rng) % bidders.len() as u64) as usize];
        let result = env.app.execute_contract(
            Addr::unchecked(caller),
            env.auction_addr.clone(),
            &ExecuteMsg::FinalizeAuction { auction_id },
            &[],
        );
        let outcome = if result.is_ok() { "ok" } else { "err" };
        trace.push(format!("finalize:{caller}:{auction_id}:{outcome}"));
    }

    for auction_id in 1..=3_u64 {
        for bidder in bidders {
            let result = env.app.execute_contract(
                Addr::unchecked(bidder),
                env.auction_addr.clone(),
                &ExecuteMsg::WithdrawBid { auction_id },
                &[],
            );
            let outcome = if result.is_ok() { "ok" } else { "err" };
            trace.push(format!("withdraw:{bidder}:{auction_id}:{outcome}"));
        }
    }

    let swap_count = 3_u32;
    for i in 0..swap_count {
        let token_id = format!("mad_replay_swap_{i}");
        mint_mad(&mut env, BIDDER3, &token_id);
        tracked_mad_tokens.push(token_id.clone());

        send_swap_deposit(&mut env, BIDDER3, &token_id);
        trace.push(format!("swap_deposit:{token_id}:ok"));
    }

    let mut pool = query_pool_contents(&env);
    pool.sort();
    let requested_ids: Vec<String> = pool.into_iter().take(swap_count as usize).collect();
    if requested_ids.len() == swap_count as usize {
        let result = env.app.execute_contract(
            Addr::unchecked(BIDDER3),
            env.auction_addr.clone(),
            &ExecuteMsg::ClaimSwap {
                requested_ids: requested_ids.clone(),
            },
            &[],
        );
        let outcome = if result.is_ok() { "ok" } else { "err" };
        trace.push(format!("claim_swap:{requested_ids:?}:{outcome}"));
    } else {
        trace.push("claim_swap:skipped_insufficient_pool".to_string());
    }

    let snapshot = canonical_state_snapshot(&env, &cosmic_token_ids, &tracked_mad_tokens);
    let hash = stable_fnv1a_64(&snapshot);
    ReplayResult {
        trace,
        snapshot,
        hash,
    }
}

fn write_replay_artifacts(
    artifact_dir: &str,
    seed: u64,
    run_id: &str,
    replay: &ReplayResult,
) -> std::io::Result<()> {
    let dir = PathBuf::from(artifact_dir);
    fs::create_dir_all(&dir)?;

    let base = format!("seed_{seed}_run_{run_id}");
    let trace_path = dir.join(format!("{base}_trace.log"));
    let snapshot_path = dir.join(format!("{base}_snapshot.log"));
    let meta_path = dir.join(format!("{base}_meta.txt"));

    let mut trace_body = String::new();
    for line in &replay.trace {
        writeln!(&mut trace_body, "{line}").unwrap();
    }
    fs::write(trace_path, trace_body)?;
    fs::write(snapshot_path, &replay.snapshot)?;
    fs::write(
        meta_path,
        format!("seed={seed}\nrun_id={run_id}\nhash={}\n", replay.hash),
    )?;
    Ok(())
}

#[test]
fn determinism_replay_same_seed_same_state_hash() {
    let seed = 12_345_678_u64;
    let run_a = run_deterministic_replay(seed);
    let run_b = run_deterministic_replay(seed);

    assert_eq!(run_a.trace, run_b.trace);
    assert_eq!(run_a.snapshot, run_b.snapshot);
    assert_eq!(run_a.hash, run_b.hash);
}

#[test]
fn determinism_replay_multi_seed_stable() {
    let seeds = [1_u64, 42, 7777, 20260228];
    for seed in seeds {
        let run_a = run_deterministic_replay(seed);
        let run_b = run_deterministic_replay(seed);
        assert_eq!(run_a.trace, run_b.trace, "trace mismatch for seed {seed}");
        assert_eq!(
            run_a.snapshot, run_b.snapshot,
            "snapshot mismatch for seed {seed}"
        );
        assert_eq!(run_a.hash, run_b.hash, "hash mismatch for seed {seed}");
    }
}

#[test]
fn determinism_replay_emit_seed_snapshot() {
    let seed = std::env::var("DETERMINISM_SEED")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);
    let run_id = std::env::var("DETERMINISM_RUN_ID").unwrap_or_else(|_| "manual".to_string());
    let replay = run_deterministic_replay(seed);

    if let Ok(artifact_dir) = std::env::var("DETERMINISM_ARTIFACT_DIR") {
        write_replay_artifacts(&artifact_dir, seed, &run_id, &replay)
            .expect("failed to write determinism artifacts");
    }

    println!("DETERMINISM_SEED={seed}");
    println!("DETERMINISM_RUN_ID={run_id}");
    println!("DETERMINISM_HASH={}", replay.hash);
}
