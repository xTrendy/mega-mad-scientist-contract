# Cosmic Mad Scientist Smart Contract

CosmWasm smart contract for the Cosmic Mad Scientist NFT auction system on Cosmos Hub.

## Architecture

**Auction Module** — Users bid Standard Mad Scientist NFTs on 1-of-1 Cosmic Mad Scientist NFTs. Highest bid (by NFT count) wins. Includes anti-sniping protection that extends auctions when last-minute bids arrive. Time-based tie-breaking: to take the lead you must strictly exceed the current highest bid.

**Swap Pool** — Winning bids' NFTs flow into a community swap pool. Any holder can swap their Standard Mad Scientists 1:1 with pool contents.

## Build

```bash
# Install wasm target
rustup target add wasm32-unknown-unknown

# Build optimized wasm
cargo wasm

# Run tests
cargo test

# Generate JSON schema
cargo schema
```

## Optimized Build (for deployment)

```bash
docker run --rm -v "$(pwd)":/code \
  --mount type=volume,source="$(basename "$(pwd)")_cache",target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/optimizer:0.17.0
```

## One-Command Rehearsal

Run the high-signal pre-launch suite (5 simultaneous Cosmic auctions, no double dip, atomicity checks, edge-time checks, multi-auction fuzz invariant):

```bash
./rehearse.sh
```

Run the same suite plus full test pass:

```bash
./rehearse.sh --full
```

## Security Stress Suite

Run mutation checks, long-run adversarial fuzzing, and DoS-style stress tests:

```bash
./security_stress.sh
```

Optional: increase fuzz intensity:

```bash
PROPTEST_CASES=2000 ./security_stress.sh
```

## Repeat Regression Suite (P0 Determinism/Flake Gate)

Run seeded determinism replay repeatedly and repeat critical integration tests. This writes logs and replay artifacts to `logs/ci_repeat/`.

```bash
./ci_repeat_regression.sh
```

Optional: tune runtime intensity.

```bash
ITERATIONS=50 CRITICAL_REPEAT=20 ./ci_repeat_regression.sh
```

## Real Local-Chain E2E Harness (wasmd)

Runs a live on-chain flow against a running local `wasmd` node:

- store/instantiate `cw721-base` + auction contracts
- mint Standard/Cosmic NFTs
- create auction via CW721 `send_nft` hook
- place bids, finalize, loser withdraw
- swap staging + claim

```bash
# Required:
# 1) local wasmd node is already running and funded keys exist
# 2) cw721-base wasm path is available
WASM_RUST_TOOLCHAIN=stable \
CW721_WASM=/absolute/path/to/cw721_base.wasm \
./localnet_e2e.sh
```

Useful overrides:

```bash
WASM_RUST_TOOLCHAIN=stable \
CHAIN_ID=localwasm \
NODE=http://127.0.0.1:26657 \
WASMD_HOME=$HOME/.wasmd \
ADMIN_KEY=admin BIDDER1_KEY=bidder1 BIDDER2_KEY=bidder2 \
CW721_WASM=/absolute/path/to/cw721_base.wasm \
./localnet_e2e.sh
```

## One-Command Two-Node Localnet + E2E

Boots a 2-node local `wasmd` network, runs full E2E, then shuts it down.

```bash
WASM_RUST_TOOLCHAIN=stable ./localnet_two_node_e2e.sh
```

By default, this script auto-downloads a prebuilt, validator-compatible `cw721_base.wasm` release artifact pinned to `public-awesome/cw-nfts v0.21.0` (matching this repo's CW721/CosmWasm dependency line). Override with `CW721_WASM=/path/to/cw721_base.wasm` or `CW721_RELEASE_URL=...` if needed.

For auction wasm, the script prefers an optimizer-built artifact at `artifacts/mega_mad_scientist.wasm` when available (and will auto-attempt a Docker optimizer build if Docker exists). If no optimizer artifact is available, it falls back to local Rust build.

Optional explicit optimizer build:

```bash
docker run --rm -v "$(pwd)":/code \
  --mount type=volume,source=cosmwasm_target_cache,target=/target \
  --mount type=volume,source=cosmwasm_registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/optimizer:0.17.0

AUCTION_WASM=artifacts/mega_mad_scientist.wasm \
FORCE_AUCTION_REBUILD=false \
WASM_RUST_TOOLCHAIN=stable \
./localnet_two_node_e2e.sh
```

This is local-only testing. It does not touch Cosmos mainnet or testnet unless you explicitly point scripts at remote RPC endpoints.

## Contract Messages

### Instantiate
```json
{
  "admin": "cosmos1...",
  "mad_scientist_collection": "cosmos1...",
  "mega_mad_scientist_collection": "cosmos1...",
  "default_min_bid": 1,
  "anti_snipe_window": 300,
  "anti_snipe_extension": 300
}
```

Note: documentation uses "Standard" and "Cosmic" terminology, while some wire/API field names remain legacy for backward compatibility (for example `mega_mad_scientist_collection`, `deposit_mega`).

### Execute

| Message | Who | Description |
|---------|-----|-------------|
| `FinalizeAuction` | Anyone (after end) | End auction, distribute NFTs |
| `CancelAuction` | Admin | Cancel auction before any bids are placed |
| `WithdrawBid` | Losing bidder | Withdraw escrowed Standard NFTs after finalize/cancel |
| `ClaimSwap` | Any holder | Complete 1:1 swap using previously staged NFTs |
| `WithdrawStaged` | Any holder | Return currently staged swap NFTs to self |
| `SetPaused` | Admin | Pause/unpause incoming NFT operations |
| `ProposeAdmin` | Admin | Propose a new admin (step 1 of 2) |
| `AcceptAdmin` | Pending admin | Accept admin role (step 2 of 2) |
| `ForceCompleteAuction` | Admin | Force-complete a finalizing auction |
| `UpdateConfig` | Admin | Update contract settings |
| `ReceiveNft` | CW721 hook | Routes incoming NFTs |

### Query

| Query | Returns |
|-------|---------|
| `GetConfig` | Contract configuration |
| `GetAuction` | Auction details + bids |
| `GetAllAuctions` | Filtered/paginated auction list |
| `GetBids` | Bids for an auction |
| `GetUserBid` | Specific user's bid |
| `GetPoolContents` | Token IDs in swap pool |
| `GetPoolSize` | Pool NFT count |
| `GetSwapStaging` | Token IDs the user has staged for swap |

## NFT Integration

The contract receives NFTs via the CW721 `Send` mechanism. Embed a `ReceiveNftAction` in the `msg` field:

**To bid on an auction** (send Standard Mad Scientists):
```json
{ "bid": { "auction_id": 1 } }
```

**To deposit a Cosmic for auction** (send Cosmic Mad Scientist, admin only):
```json
{ "deposit_mega": { "start_time": 1700000000, "end_time": 1700086400, "min_bid": 2 } }
```

**To stage a Standard NFT for swap**:
```json
"swap_deposit"
```

Then call execute:
```json
{ "claim_swap": { "requested_ids": ["mad-42", "mad-73"] } }
```

## Security Notes

- All bidders' NFTs are escrowed in the contract during auctions
- Losing bidders can withdraw escrowed NFTs after finalize/cancel
- Reentrancy guards via CosmWasm's single-threaded execution model
- Duplicate token ID detection in bids and swaps
- Mandatory audit recommended before mainnet deployment

### Odin Scan CI (every push)

This repo includes `.github/workflows/odin-scan.yml` to run Odin Scan on every push and pull request.

Required one-time setup:

1. Install/connect Odin Scan to the GitHub repo.
2. Add repository secret `ODIN_SCAN_API_KEY` in GitHub Actions secrets.

The workflow is configured to fail on `high` or `critical` findings and upload SARIF/artifacts.
