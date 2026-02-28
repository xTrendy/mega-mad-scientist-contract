#!/usr/bin/env bash
# One-command rehearsal suite for pre-testnet / pre-mainnet confidence.
# Runs the highest-signal scenarios:
# - 5 simultaneous Cosmic auctions + no double dipping
# - Finalize/withdraw failure atomicity
# - Boundary-time bidding and anti-snipe edge behavior
# - Multi-auction global token-lock fuzz invariant

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

run() {
  echo ""
  echo "+ $*"
  "$@"
}

FULL=false
if [[ "${1:-}" == "--full" ]]; then
  FULL=true
fi

echo "== Cosmic Mad Scientist Rehearsal =="
echo "Working dir: $ROOT_DIR"
if [[ "$FULL" == "true" ]]; then
  echo "Mode: full (targeted suite + full cargo test)"
else
  echo "Mode: targeted"
fi

run cargo fmt --check
run cargo clippy --all-targets --all-features -- -D warnings

# Critical integration scenarios
run cargo test --test integration integration_five_simultaneous_megas_no_double_dip
run cargo test --test integration integration_finalize_is_atomic_when_mega_transfer_fails
run cargo test --test integration integration_withdraw_is_atomic_when_mad_transfer_fails

# Critical edge and invariant checks
run cargo test test_bid_exactly_at_start_time_allowed
run cargo test test_bid_exactly_at_end_time_allowed
run cargo test test_anti_snipe_triggers_at_exact_window_boundary
run cargo test proptest_multi_auction_token_lock_invariants

if [[ "$FULL" == "true" ]]; then
  run cargo test
fi

echo ""
echo "Rehearsal suite passed."
