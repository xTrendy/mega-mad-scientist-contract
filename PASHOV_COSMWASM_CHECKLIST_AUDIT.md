# Pashov -> CosmWasm Checklist Audit

Date: 2026-03-05  
Project: `mega-mad-scientist-contract` (CosmWasm/Rust)  
Source baseline: `pashov/skills` Solidity attack-vector set (adapted for CosmWasm semantics)

## Scope

- Contract logic:
  - `src/contract.rs`
  - `src/msg.rs`
  - `src/state.rs`
  - `src/error.rs`
- Test evidence:
  - `src/tests.rs`
  - `tests/integration.rs`
- Validation run:
  - `cargo test --locked` -> `78` unit tests + `18` integration tests passed

## CosmWasm-Adapted Checklist (from Pashov vectors)

`PASS` Access control on privileged state changes
- Admin-gated flows enforced for cancel/pause/admin-transfer/force-complete/update-config.
- Refs: `src/contract.rs:622`, `src/contract.rs:836`, `src/contract.rs:858`, `src/contract.rs:902`, `src/contract.rs:932`.

`PASS` Callback entrypoint gating (collection + sender routing)
- `ReceiveNft` validates expected collection contract and routes only expected actions.
- Refs: `src/contract.rs:199`, `src/contract.rs:214`, `src/contract.rs:230`.

`PASS` Unexpected native-funds rejection
- Execute rejects attached native funds globally.
- Refs: `src/contract.rs:124`, `src/error.rs:108`.
- Test: `tests/integration.rs:1147`.

`PASS` Cross-auction double-dip prevention
- Per-token escrow lock prevents same Standard NFT in multiple active auctions.
- Refs: `src/contract.rs:343`, `src/state.rs:122`.
- Tests: `tests/integration.rs:606`, `src/tests.rs:595`.

`PASS` Atomicity of external NFT transfers
- Critical transfer paths are message-based and transaction-atomic.
- Tests prove revert-on-failure behavior:
  - Finalize atomicity: `tests/integration.rs:703`
  - Withdraw atomicity: `tests/integration.rs:746`

`PASS` Invariant separation for winner vs loser escrow
- Winner escrow moves to pool on finalize; losers self-withdraw later.
- Refs: `src/contract.rs:501`, `src/contract.rs:551`.

`PASS` State-machine guardrails for auction lifecycle
- Explicit status checks for active/finalizing/cancelled/completed transitions.
- Refs: `src/contract.rs:475`, `src/contract.rs:564`, `src/contract.rs:910`.

`PASS` Swap integrity checks
- Enforces non-empty, 1:1 count, duplicate detection, overlap prevention, pool-membership checks.
- Refs: `src/contract.rs:717`, `src/contract.rs:722`, `src/contract.rs:730`, `src/contract.rs:740`, `src/contract.rs:750`.

`PASS` Duplicate-auction prevention for the same Cosmic token
- Active-auction guard map for deposited Cosmic token IDs.
- Refs: `src/contract.rs:264`, `src/state.rs:125`.

`PASS` Pause behavior does not block exits
- Pause blocks inbound `ReceiveNft` paths while withdrawals remain available.
- Refs: `src/contract.rs:184`, `src/tests.rs:1651`.

`PASS` End-time boundary policy is explicit and deterministic
- Bidding closes at `now >= end_time`; finalize can execute at `now >= end_time`.
- Refs: `src/contract.rs:337`, `src/contract.rs:480`.
- Tests: `src/tests.rs` (`test_bid_exactly_at_end_time_rejected`, `test_finalize_exactly_at_end_time_allowed`).

`PASS` Loop boundedness enforced by config validation
- Core loops exist in finalize/withdraw/swap flows.
- Refs: `src/contract.rs:503`, `src/contract.rs:582`, `src/contract.rs:763`, `src/contract.rs:805`.
- Limits now require `>= 1` and are capped with explicit upper bounds in both instantiate and update paths.
- Refs: `src/contract.rs` (bounded validation helper + instantiate/update validation), `src/msg.rs:25`, `src/msg.rs:27`, `src/msg.rs:29`.
- Tests: `src/tests.rs` (`test_*_zero_rejected`, `test_instantiate_rejects_limits_above_cap`, `test_update_config_rejects_limits_above_cap`).

`PASS` Query error propagation consistency
- `query_all_auctions` now propagates storage/deserialize errors from range iteration.
- Refs: `src/contract.rs` (`query_all_auctions` loop uses `item?`).
- Test: `src/tests.rs` (`test_query_all_auctions_propagates_deserialization_errors`).

## Resolved Findings (Patched 2026-03-05)

### [P2] Config could disable DoS bounds on loop-heavy paths

Where:
- Config API semantics: `src/msg.rs:25`, `src/msg.rs:27`, `src/msg.rs:29`
- Runtime update path now validates non-zero bounded values.
- Loop-heavy executors: `src/contract.rs:503`, `src/contract.rs:582`, `src/contract.rs:763`, `src/contract.rs:805`

Resolution:
- Added bounded validation helper and enforced limits in instantiate/update config.
- Added regression tests for zero and above-cap values.

Status: `RESOLVED`

---

### [P3] End-time boundary allows bid/finalize race at exact second

Where:
- Bid rejection check: `src/contract.rs:337` (`now >= end_time`)
- Finalize check: `src/contract.rs:480` (`now < end_time`)

Resolution:
- Standardized policy: end timestamp is exclusive for bids, inclusive for finalize.
- Added regression tests for exact-end behavior.

Status: `RESOLVED`

---

### [P3] `query_all_auctions` masks iterator errors

Where:
- Old logic used `r.ok().and_then(...)` and could silently skip bad rows.

Resolution:
- Replaced with explicit iteration and `item?` error propagation.
- Added a regression test that corrupts one auction row and asserts query now errors.

Status: `RESOLVED`

## Priority Next Steps

1. Re-run localnet/two-node E2E after these logic changes and archive logs.
2. Run testnet smoke rehearsal using your deploy/runbook scripts.
3. Keep caps conservative in production config (do not max them out unless operationally required).
