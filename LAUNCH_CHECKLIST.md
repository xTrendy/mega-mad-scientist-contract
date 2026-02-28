# Cosmic Mad Scientist Contract Launch Checklist

## 1. Code Freeze And Build Gates

- [ ] Freeze code and dependencies (no new feature PRs).
- [ ] Run `./rehearse.sh` (or `./rehearse.sh --full`).
- [ ] Run `./security_stress.sh` (or `PROPTEST_CASES=2000 ./security_stress.sh`).
- [ ] Run `./ci_repeat_regression.sh` (or tuned `ITERATIONS=50 CRITICAL_REPEAT=20 ./ci_repeat_regression.sh`).
- [ ] Run `cargo fmt`.
- [ ] Run `cargo test`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo deny check`.
- [ ] Record commit hash and compiled wasm artifact hash.

## 2. Config And Admin Safety Checks

- [ ] Confirm `admin` is the intended production multisig.
- [ ] Confirm `mad_scientist_collection` address is correct.
- [ ] Confirm `mega_mad_scientist_collection` address is correct.
- [ ] Confirm the two collection addresses are different.
- [ ] Confirm `default_min_bid` is final.
- [ ] Confirm anti-snipe settings are final:
- [ ] `anti_snipe_window`
- [ ] `anti_snipe_extension`
- [ ] `max_extension`
- [ ] Confirm limits are final:
- [ ] `max_bidders_per_auction`
- [ ] `max_staging_size`
- [ ] `max_nfts_per_bid`
- [ ] Confirm `paused = false` before launch.
- [ ] Confirm two-step admin transfer works (`ProposeAdmin` and `AcceptAdmin`).

## 3. Testnet Rehearsal (Must Pass)

- [ ] Create 5 simultaneous Cosmic auctions (same start/end window).
- [ ] One wallet wins all 5 Cosmic auctions with distinct Standard Mad NFTs.
- [ ] Attempt to reuse the same Standard Mad NFT across auctions; verify it fails.
- [ ] Finalize all auctions after end time.
- [ ] Verify each Cosmic NFT lands with the correct winner.
- [ ] Verify winners' Standard Mad NFTs enter pool and pool size matches expected count.
- [ ] Verify losers can withdraw after `Finalizing`.
- [ ] Verify no-bid finalize returns Cosmic to original depositor.
- [ ] Verify cancel before bids returns Cosmic to original depositor.
- [ ] Verify force-complete path still allows loser withdrawals.
- [ ] Verify swap flow (`SwapDeposit` -> `ClaimSwap`) with 1:1 count enforcement.
- [ ] Verify `WithdrawStaged` returns staged NFTs.
- [ ] Verify `SetPaused { paused: true }` blocks new deposits/bids.
- [ ] Verify attaching native funds to execute calls is rejected.

## 4. Launch-Day Execution

- [ ] Deploy contract from audited commit hash only.
- [ ] Verify on-chain config immediately after instantiate.
- [ ] Create first controlled auction with internal wallet.
- [ ] Place/withdraw/finalize one dry-run auction on production chain.
- [ ] Open public access only after dry-run matches expected behavior.

## 5. Post-Launch Monitoring (First 24 Hours)

- [ ] Monitor failed execute calls and root-cause messages.
- [ ] Track each auction status transition (`Active` -> `Finalizing`/`Completed`).
- [ ] Track winner assignment and Cosmic transfer destination per auction.
- [ ] Track loser withdraw success rates and pending escrow counts.
- [ ] Track pool size growth against expected winner bid totals.

## 6. Emergency Response Runbook

- [ ] Trigger `SetPaused { paused: true }` immediately on anomaly.
- [ ] Disable frontend actions that submit `ReceiveNft` calls.
- [ ] Snapshot all active auctions, bids, escrow, and pool state.
- [ ] Classify incident:
- [ ] User error / UI issue
- [ ] Config error
- [ ] Contract logic issue
- [ ] For stuck `Finalizing` auctions, use `ForceCompleteAuction` only after review.
- [ ] Communicate incident details and timeline publicly.
- [ ] Patch, re-test on testnet, then unpause only after verification.
