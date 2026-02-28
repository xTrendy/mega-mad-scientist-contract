use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};

/// Top-level contract configuration
#[cw_serde]
pub struct Config {
    /// Contract administrator
    pub admin: Addr,
    /// Pending admin for two-step transfer (None if no transfer in progress)
    pub pending_admin: Option<Addr>,
    /// Whether the contract is paused (blocks all ReceiveNft operations)
    pub paused: bool,
    /// CW721 contract address for Standard Mad Scientist NFTs
    pub mad_scientist_collection: Addr,
    /// CW721 contract address for Cosmic Mad Scientist NFTs
    pub mega_mad_scientist_collection: Addr,
    /// Default minimum bid (number of Standard Mad Scientists)
    pub default_min_bid: u64,
    /// Anti-sniping window in seconds. If a bid arrives within this many
    /// seconds of the auction end, the end time is extended by this duration.
    pub anti_snipe_window: u64,
    /// Anti-sniping extension in seconds (how much time to add)
    pub anti_snipe_extension: u64,
    /// Maximum total extension allowed beyond original_end_time (caps anti-snipe griefing).
    /// Defaults to 86400 (24 hours).
    pub max_extension: u64,
    /// Maximum unique bidders per auction (0 = unlimited). Prevents griefing.
    pub max_bidders_per_auction: u64,
    /// Maximum tokens a user can stage for swapping (0 = unlimited). Prevents unbounded storage.
    pub max_staging_size: u64,
    /// Maximum NFTs a single bidder can escrow per auction (0 = unlimited). Prevents storage bloat.
    pub max_nfts_per_bid: u64,
}

/// Status of an auction
#[cw_serde]
pub enum AuctionStatus {
    Active,
    Completed,
    Cancelled,
    /// Auction ended, winner determined, but losers still need to withdraw.
    /// Transitions to Completed once all losers have withdrawn (or admin force-completes).
    Finalizing,
}

/// An auction for a single Cosmic Mad Scientist NFT
#[cw_serde]
pub struct Auction {
    pub auction_id: u64,
    /// Address that deposited the Cosmic Mad Scientist NFT that this auction is selling
    pub depositor: Addr,
    /// Token ID of the Cosmic Mad Scientist NFT being auctioned
    pub mega_token_id: String,
    /// Block time when auction starts (seconds)
    pub start_time: u64,
    /// Block time when auction ends (seconds) — may be extended by anti-sniping
    pub end_time: u64,
    /// Original end time before any anti-snipe extensions
    pub original_end_time: u64,
    pub status: AuctionStatus,
    /// Minimum number of Standard Mad Scientists required to bid
    pub min_bid: u64,
    /// Current highest bid quantity
    pub highest_bid_count: u64,
    /// Address of the current highest bidder (None if no bids)
    pub highest_bidder: Option<Addr>,
    /// Timestamp of the highest bid (used for time-based tie-breaking)
    pub highest_bid_time: u64,
    /// Total number of unique bidders (any escrow deposit counts)
    pub total_bidders: u64,
}

/// A bid placed by a user on a specific auction
#[cw_serde]
pub struct Bid {
    pub bidder: Addr,
    /// Standard Mad Scientist token IDs offered in this bid
    pub token_ids: Vec<String>,
    /// Timestamp when this bid was placed
    pub timestamp: u64,
}

/// Record of a swap transaction for transparency (emitted as attributes, not stored)
#[cw_serde]
pub struct SwapRecord {
    pub swapper: Addr,
    /// Token IDs the swapper sent into the pool
    pub offered_ids: Vec<String>,
    /// Token IDs the swapper received from the pool
    pub received_ids: Vec<String>,
    /// Block time of the swap
    pub timestamp: u64,
}

// ── Storage keys ──────────────────────────────────────────────────────

pub const CONFIG: Item<Config> = Item::new("config");

/// Auto-incrementing auction ID counter
pub const NEXT_AUCTION_ID: Item<u64> = Item::new("next_auction_id");

/// auction_id -> Auction
pub const AUCTIONS: Map<u64, Auction> = Map::new("auctions");

/// (auction_id, bidder_addr) -> Bid
pub const BIDS: Map<(u64, &Addr), Bid> = Map::new("bids");

/// Set of token IDs currently in the swap pool.
/// We store token_id -> empty tuple for O(1) lookups.
pub const POOL: Map<&str, ()> = Map::new("pool");

/// O(1) pool size counter (Fix #8)
pub const POOL_SIZE: Item<u64> = Item::new("pool_size");

/// Tracks all token IDs escrowed per (auction_id, bidder) for safe return.
/// This is redundant with BIDS but kept for explicit escrow accounting.
pub const ESCROW: Map<(u64, &Addr), Vec<String>> = Map::new("escrow");

/// token_id -> auction_id for Mad Scientist NFTs currently escrowed as bids.
/// Prevents the same NFT from being used in multiple auctions at once.
pub const ESCROWED_BID_TOKENS: Map<&str, u64> = Map::new("escrowed_bid_tokens");

/// Maps Cosmic token_id -> active auction_id to prevent duplicate auctions (Fix #6)
pub const ACTIVE_MEGA_AUCTIONS: Map<&str, u64> = Map::new("active_mega");
