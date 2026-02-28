use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Addr;
use cw721::Cw721ReceiveMsg;

use crate::state::{Auction, AuctionStatus, Bid};

// ── Instantiate ───────────────────────────────────────────────────────

#[cw_serde]
pub struct InstantiateMsg {
    /// Optional admin; defaults to message sender
    pub admin: Option<String>,
    /// CW721 contract address for Standard Mad Scientists
    pub mad_scientist_collection: String,
    /// CW721 contract address for Cosmic Mad Scientists
    pub mega_mad_scientist_collection: String,
    /// Default minimum bid count (defaults to 1)
    pub default_min_bid: Option<u64>,
    /// Anti-sniping window in seconds (defaults to 300 = 5 minutes)
    pub anti_snipe_window: Option<u64>,
    /// Anti-sniping extension in seconds (defaults to 300 = 5 minutes)
    pub anti_snipe_extension: Option<u64>,
    /// Maximum total extension beyond original end time (defaults to 86400 = 24 hours)
    pub max_extension: Option<u64>,
    /// Maximum unique bidders per auction (defaults to 100, 0 = unlimited)
    pub max_bidders_per_auction: Option<u64>,
    /// Maximum tokens a user can stage for swapping (defaults to 50, 0 = unlimited)
    pub max_staging_size: Option<u64>,
    /// Maximum NFTs a single bidder can escrow per auction (defaults to 50, 0 = unlimited)
    pub max_nfts_per_bid: Option<u64>,
}

// ── Execute ───────────────────────────────────────────────────────────

#[cw_serde]
pub enum ExecuteMsg {
    /// End an auction, transfer Cosmic to winner, mark as Finalizing.
    /// Losers must call WithdrawBid to reclaim their NFTs.
    /// Can be called by anyone after the auction end time.
    FinalizeAuction { auction_id: u64 },

    /// Cancel an auction (admin only). Only allowed before any bids are placed.
    CancelAuction { auction_id: u64 },

    /// Losers (or anyone whose bid was not the winning bid) can withdraw
    /// their escrowed NFTs after the auction is finalized or cancelled.
    WithdrawBid { auction_id: u64 },

    /// Swap Standard Mad Scientists 1:1 with ones in the pool.
    /// The caller must first SendNft their offered tokens to this contract
    /// with a ReceiveNftAction::SwapDeposit message. Once all offered tokens
    /// are deposited, call this to complete the swap by specifying which
    /// pool tokens you want.
    ClaimSwap { requested_ids: Vec<String> },

    /// Return all tokens currently staged by the caller back to caller.
    /// Useful when a user changes their mind or cannot complete a swap.
    WithdrawStaged {},

    /// Pause or unpause the contract (admin only).
    /// When paused, no new ReceiveNft operations are accepted.
    SetPaused { paused: bool },

    /// Step 1 of two-step admin transfer: propose a new admin (admin only).
    ProposeAdmin { new_admin: String },

    /// Step 2 of two-step admin transfer: the proposed admin accepts (pending admin only).
    AcceptAdmin {},

    /// Admin force-completes a Finalizing auction (admin only).
    /// Use when losers refuse to withdraw and the auction is stuck.
    ForceCompleteAuction { auction_id: u64 },

    /// Update contract configuration (admin only).
    /// NOTE: admin transfer now uses ProposeAdmin/AcceptAdmin.
    UpdateConfig {
        default_min_bid: Option<u64>,
        anti_snipe_window: Option<u64>,
        anti_snipe_extension: Option<u64>,
        max_extension: Option<u64>,
        max_bidders_per_auction: Option<u64>,
        max_staging_size: Option<u64>,
        max_nfts_per_bid: Option<u64>,
    },

    /// CW721 receive hook — routes incoming NFTs to the right handler.
    ReceiveNft(Cw721ReceiveMsg),
}

/// Inner message embedded in CW721 Send's `msg` field to route NFTs.
#[cw_serde]
pub enum ReceiveNftAction {
    /// Bid on an auction with this NFT (send one at a time, cumulative)
    Bid { auction_id: u64 },
    /// Deposit a Cosmic Mad Scientist for a new auction (admin only)
    DepositMega {
        start_time: u64,
        end_time: u64,
        min_bid: Option<u64>,
    },
    /// Deposit a Standard Mad Scientist into the swap staging area
    SwapDeposit,
}

// ── Query ─────────────────────────────────────────────────────────────

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Get contract config
    #[returns(ConfigResponse)]
    GetConfig {},

    /// Get a single auction by ID
    #[returns(AuctionResponse)]
    GetAuction { auction_id: u64 },

    /// List auctions, optionally filtered by status
    #[returns(AuctionsResponse)]
    GetAllAuctions {
        status: Option<AuctionStatus>,
        start_after: Option<u64>,
        limit: Option<u32>,
    },

    /// Get all bids for a specific auction
    #[returns(BidsResponse)]
    GetBids {
        auction_id: u64,
        start_after: Option<String>,
        limit: Option<u32>,
    },

    /// Get a specific user's bid on an auction
    #[returns(BidResponse)]
    GetUserBid { auction_id: u64, bidder: String },

    /// Get all token IDs currently in the swap pool
    #[returns(PoolContentsResponse)]
    GetPoolContents {
        start_after: Option<String>,
        limit: Option<u32>,
    },

    /// Get the number of NFTs in the swap pool
    #[returns(PoolSizeResponse)]
    GetPoolSize {},

    /// Get tokens a user has staged for swapping
    #[returns(SwapStagingResponse)]
    GetSwapStaging { user: String },
}

// ── Query Responses ───────────────────────────────────────────────────

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Addr,
    pub pending_admin: Option<Addr>,
    pub paused: bool,
    pub mad_scientist_collection: Addr,
    pub mega_mad_scientist_collection: Addr,
    pub default_min_bid: u64,
    pub anti_snipe_window: u64,
    pub anti_snipe_extension: u64,
    pub max_extension: u64,
    pub max_bidders_per_auction: u64,
    pub max_staging_size: u64,
    pub max_nfts_per_bid: u64,
}

#[cw_serde]
pub struct AuctionResponse {
    pub auction: Auction,
    pub bids: Vec<Bid>,
}

#[cw_serde]
pub struct AuctionsResponse {
    pub auctions: Vec<Auction>,
}

#[cw_serde]
pub struct BidsResponse {
    pub bids: Vec<Bid>,
}

#[cw_serde]
pub struct BidResponse {
    pub bid: Option<Bid>,
}

#[cw_serde]
pub struct PoolContentsResponse {
    pub token_ids: Vec<String>,
}

#[cw_serde]
pub struct PoolSizeResponse {
    pub size: u64,
}

#[cw_serde]
pub struct SwapStagingResponse {
    pub token_ids: Vec<String>,
}
