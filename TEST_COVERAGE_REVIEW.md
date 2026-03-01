# Test Coverage Review (Localnet E2E)

## 1) What Run We Are Talking About

This review covers the expanded local-chain E2E suite using:

```bash
AUCTION_WASM=artifacts/mega_mad_scientist.wasm \
FORCE_AUCTION_REBUILD=false \
WASM_RUST_TOOLCHAIN=stable \
RESET_LOCALNET=true \
./localnet_two_node_e2e.sh
```

Prerequisite: `artifacts/mega_mad_scientist.wasm` must already exist (prefer build via `cosmwasm/optimizer:0.17.0`). Source-built wasm (`FORCE_AUCTION_REBUILD=true`) may still be rejected on strict validators with feature-gate errors (for example, bulk-memory support), so optimizer output is the reliable path.

This run is local-only and does not interact with public chains.

Last updated: 2026-03-01 (expanded from happy-path-only to broad branch coverage).
Provenance: this document mirrors assertions scripted in `localnet_e2e.sh`. For audit trail, capture a fresh run log with `./localnet_two_node_e2e.sh 2>&1 | tee run_$(date +%s).log` and record `git rev-parse HEAD`.

## 2) Exactly What Was Tested (And Passed)

Source of truth for this flow: `localnet_e2e.sh`. To produce an audit-grade evidence log, run with `2>&1 | tee` and record the git commit hash (`git rev-parse HEAD`) at time of run.

`PASS` Network and deployment path:
- 2-node local `wasmd` network booted.
- RPC readiness and first-block readiness checks passed.
- `cw721-base` wasm stored and instantiated twice (Standard + Cosmic collections).
- auction wasm stored and instantiated.

`PASS` Auction happy-path (auction 1):
- Cosmic NFT minted to admin.
- Standard NFTs minted to bidder wallets.
- Auction created via CW721 `send_nft` receive-hook (`deposit_mega`).
- Five bid deposits sent via CW721 `send_nft` receive-hook (`bid`).
- Auction finalized after end-time buffer.
- Winner field asserted (`highest_bidder` equals expected bidder).
- Auction status asserted (`finalizing`).

`PASS` P1 — Cosmic winner ownership assertion:
- `owner_of(cosmic-1)` queried post-finalize and asserted to be the winner.

`PASS` P1 — Pool token-ID membership assertion:
- `get_pool_contents` queried and exact token set asserted (`mad-3,mad-4,mad-5`).

`PASS` Winner bid to pool transfer behavior:
- Pool size asserted after finalize (`size = 3`), matching winner bid count.

`PASS` Loser withdrawal:
- Losing bidder called `withdraw_bid`.
- Ownership of loser NFTs checked and confirmed returned to loser.

`PASS` Swap behavior:
- User staged two NFTs via receive-hook (`swap_deposit`).
- User executed `claim_swap` for two pool NFTs.
- Ownership of requested pool NFTs checked and confirmed transferred to claimer.

`PASS` P2a — No-bid finalize (auction 2):
- Auction created with already-passed end time.
- Finalized with no bids.
- Status asserted `completed`.
- Cosmic NFT asserted returned to depositor (admin).

`PASS` P2b — Cancel before bids (auction 3):
- Auction created, cancelled immediately (no bids placed).
- Status asserted `cancelled`.
- Cosmic NFT asserted returned to depositor (admin).

`PASS` P2c — Cancel after bids fails (auction 4):
- Auction created, one bid placed.
- Cancel attempted → transaction failed as expected.
- Error message asserted to contain "bids have already been placed".

`PASS` P2d — Pause / unpause:
- Contract paused via `set_paused`.
- Paused flag asserted `true` via config query.
- Bid attempted while paused → transaction failed as expected.
- Error message asserted to contain "paused".
- Contract unpaused, same bid succeeded.

`PASS` P2e — Force-complete (auction 4):
- Auction with bids finalized → status `finalizing`.
- `force_complete_auction` called → status `completed`.
- Cosmic NFT asserted transferred to winner.
- Winner `withdraw_bid` attempt asserted to fail with "No escrowed NFTs found" (winner bid escrow correctly not withdrawable).

`PASS` P2f — Withdraw-staged:
- Two NFTs staged via `swap_deposit`.
- `get_swap_staging` asserted 2 tokens.
- `withdraw_staged` called.
- Ownership of both NFTs asserted returned to user.
- Staging asserted empty after withdrawal.

`PASS` P2g — Funds rejection:
- `finalize_auction` called with attached funds → transaction failed.
- Error message asserted to contain "does not accept funds".

`PASS` P2h — Admin transfer (propose + accept):
- `propose_admin` called, pending admin asserted via config query.
- `accept_admin` called, admin asserted changed.
- Round-trip transfer back to original admin verified.

`PASS` P3 — Multi-auction with cross-auction double-dip prevention:
- 3 simultaneous auctions created (auctions 5, 6, 7) with overlapping time windows.
- Bidder1 bid on auction 5 with multi-1, multi-2.
- Cross-auction reuse of multi-1 (already escrowed in auction 5) to auction 6 rejected.
- Bidder2 bid on auction 6 with different tokens (multi-4, multi-5, multi-6) — succeeded.
- All 3 auctions finalized.
- Auction 5 winner (bidder1) asserted, cosmic-5 ownership confirmed.
- Auction 6 winner (bidder2) asserted, cosmic-6 ownership confirmed.
- Auction 7 (no bids) completed, cosmic-7 returned to admin.
- Final pool size asserted correct (10) after all winner bids added to pool.

`PASS` P3b — Multi-bidder force-complete + loser withdraw:
- Auction 8 created, bidder1 places 1-token bid, bidder2 outbids with 2 tokens.
- Auction finalized, then force-completed.
- Losing bidder1 `withdraw_bid` succeeds and ownership of `mad-10` is asserted returned.
- Cosmic NFT (`cosmic-8`) asserted transferred to winning bidder2.

`PASS` End-to-end completion:
- Script reached terminal success line:
  - `FULL E2E local-chain test suite passed.`

## 3) What This Run Did Not Explicitly Assert

`COVERED` Cosmic owner assertion:
- Now asserted in P1 and for every auction scenario.

`COVERED` Exact pool token-ID set:
- Now asserted via `get_pool_contents` in P1.

`COVERED` Alternate lifecycle branches:
- No-bid finalize (P2a), cancel-before-bids (P2b), force-complete (P2e).
- Multi-bidder loser-withdraw-after-force-complete is explicitly demonstrated (P3b).

`COVERED` Admin/control branches:
- `SetPaused` behavior (P2d), `ProposeAdmin`/`AcceptAdmin` flow (P2h).

`NOT EXERCISED ON-CHAIN` UpdateConfig:
- `UpdateConfig` is not exercised in the localnet E2E suite. It is covered in unit tests (pure config field mutation, low wasm-boundary risk). Adding it on-chain is optional but straightforward.

`COVERED` Swap edge branches:
- `withdraw_staged` (P2f).
- Swap mismatch/duplicate/overlap failure cases covered in unit/integration tests (pure logic, low wasm-boundary risk).

`COVERED` Multi-auction contention branches:
- 3 simultaneous auctions with cross-auction double-dip attempt (P3).

`NOT COVERED` Stress/performance/fault:
- High-volume bid traffic (covered by `security_stress.sh` at Rust level).
- Long-duration soak behavior.
- Node restart/recovery behavior mid-auction.

## 4) What This Means Right Now

High confidence:
- Most high-risk contract branches with wasm-serialization-boundary risk are exercised on real local chain.
- Happy path, error paths, admin controls, multi-auction contention, and swap lifecycle all validated end-to-end.

Remaining items for launch-grade confidence:
- Repeatability runs (N consecutive localnet passes) — use `ci_repeat_regression.sh`.
- Testnet rehearsal with production-like config and operational runbook steps.

## 5) Recommended Next Test Steps

Priority 1 (done):
- ~~Add explicit assertion for Cosmic winner transfer.~~
- ~~Add explicit pool-contents assertions.~~

Priority 2 (done):
- ~~Add localnet cases for no-bid finalize, cancel, force-complete, pause/unpause, withdraw-staged.~~
- ~~Add funds rejection and admin transfer on-chain tests.~~

Priority 3 (done):
- ~~Add multi-auction localnet scenario with cross-auction token reuse attempts.~~

Priority 4 (remaining):
- Testnet rehearsal with production-like config and operational runbook steps.
- Long-duration soak test (optional, high effort).

## 6) Bottom Line

The expanded localnet E2E suite covers most high-risk contract branches where wasm serialization could diverge from native Rust behavior. All execute message variants except `UpdateConfig` are exercised on a live local chain, along with key error paths, multi-auction contention, and multi-bidder force-complete loser-withdraw behavior. `UpdateConfig` is not exercised on-chain (it is a pure config-field mutation with unit test coverage). The remaining gaps are: stress/soak behavior and testnet rehearsal.
