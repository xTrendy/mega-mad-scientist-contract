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
  cosmwasm/optimizer:0.15.0
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
| `CreateAuction` | Admin | Create auction for a Cosmic Mad Scientist |
| `PlaceBid` | Any holder | Bid Standard Mad Scientists on an auction |
| `FinalizeAuction` | Anyone (after end) | End auction, distribute NFTs |
| `CancelAuction` | Admin | Cancel auction, return all NFTs |
| `SwapFromPool` | Any holder | Swap Standard Mad Scientists 1:1 with pool |
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
| `GetSwapHistory` | Recent swap records |

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

## Security Notes

- All bidders' NFTs are escrowed in the contract during auctions
- Losers' NFTs are returned on finalize/cancel
- Reentrancy guards via CosmWasm's single-threaded execution model
- Duplicate token ID detection in bids and swaps
- Mandatory audit recommended before mainnet deployment
