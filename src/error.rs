use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized: only admin can perform this action")]
    Unauthorized {},

    #[error("Auction {auction_id} not found")]
    AuctionNotFound { auction_id: u64 },

    #[error("Auction {auction_id} is not active")]
    AuctionNotActive { auction_id: u64 },

    #[error("Auction {auction_id} has not ended yet (ends at {end_time})")]
    AuctionNotEnded { auction_id: u64, end_time: u64 },

    #[error("Auction {auction_id} has already ended")]
    AuctionAlreadyEnded { auction_id: u64 },

    #[error("Auction has not started yet (starts at {start_time})")]
    AuctionNotStarted { start_time: u64 },

    #[error("Bid of {bid_count} is below minimum of {min_bid}")]
    BidBelowMinimum { bid_count: u64, min_bid: u64 },

    #[error("Bid must exceed current highest bid of {highest}. To win a tie you must bid more, not equal.")]
    BidNotHighEnough { highest: u64 },

    #[error("Invalid start/end times: start {start_time} must be before end {end_time}")]
    InvalidAuctionTimes { start_time: u64, end_time: u64 },

    #[error("Offered and requested swap lists must be the same length")]
    SwapLengthMismatch {},

    #[error("Swap lists cannot be empty")]
    SwapEmpty {},

    #[error("Token {token_id} is not in the swap pool")]
    TokenNotInPool { token_id: String },

    #[error("Token {token_id} appears in both offered and requested lists")]
    SwapOverlap { token_id: String },

    #[error("Duplicate token ID: {token_id}")]
    DuplicateTokenId { token_id: String },

    #[error("Token {token_id} is already escrowed in auction {auction_id}")]
    TokenAlreadyEscrowedInAuction { token_id: String, auction_id: u64 },

    #[error("NFT received from unexpected collection: {collection}")]
    UnexpectedCollection { collection: String },

    #[error("Invalid receive message payload")]
    InvalidReceiveMsg {},

    #[error("No bids were placed on auction {auction_id}")]
    NoBids { auction_id: u64 },

    #[error("Caller does not own token {token_id}")]
    NotTokenOwner { token_id: String },

    #[error("Cosmic Mad Scientist token {token_id} is not held by this contract")]
    MegaNotEscrowed { token_id: String },

    #[error("A Cosmic Mad Scientist with token {token_id} already has an active auction (ID: {auction_id})")]
    DuplicateMegaAuction { token_id: String, auction_id: u64 },

    #[error("Cannot cancel auction {auction_id}: bids have already been placed. Cancel is only allowed before any bids.")]
    CannotCancelWithBids { auction_id: u64 },

    #[error("Auction {auction_id} is not in a state that allows withdrawal (must be Finalizing, Cancelled, or Completed)")]
    WithdrawNotAllowed { auction_id: u64 },

    #[error("No escrowed NFTs found for this bidder on auction {auction_id}")]
    NothingToWithdraw { auction_id: u64 },

    #[error("No tokens staged for swap. Send NFTs via CW721 Send with SwapDeposit action first.")]
    NoStagedTokens {},

    #[error("Staged {staged} tokens but requested {requested} from pool. Counts must match for 1:1 swap.")]
    SwapCountMismatch { staged: u64, requested: u64 },

    #[error("Contract is paused. No new deposits or bids are accepted.")]
    ContractPaused {},

    #[error("No pending admin transfer to accept")]
    NoPendingAdmin {},

    #[error("Only the pending admin can accept the transfer")]
    NotPendingAdmin {},

    #[error("Auction {auction_id} has reached the maximum number of bidders ({max})")]
    MaxBiddersReached { auction_id: u64, max: u64 },

    #[error("Swap staging limit reached ({max}). Claim or withdraw before staging more.")]
    StagingLimitReached { max: u64 },

    #[error("Auction {auction_id} is not in Finalizing state")]
    NotFinalizing { auction_id: u64 },

    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: String },

    #[error("This contract does not accept funds. Do not attach tokens to messages.")]
    FundsNotAllowed {},

    #[error("Collection addresses must be different (mad_scientist != mega_mad_scientist)")]
    CollectionAddressCollision {},

    #[error("Bidder has reached the maximum escrow size ({max}) for auction {auction_id}. Cannot add more NFTs.")]
    MaxEscrowPerBidder { auction_id: u64, max: u64 },
}
