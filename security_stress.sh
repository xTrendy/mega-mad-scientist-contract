#!/usr/bin/env bash
# Security stress suite:
# 1) Mutation checks
# 2) Long-run adversarial fuzzing
# 3) Gas/DoS-style stress integration tests

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

PROPTEST_CASES="${PROPTEST_CASES:-1024}"

run() {
  echo ""
  echo "+ $*"
  "$@"
}

echo "== Cosmic Mad Scientist Security Stress Suite =="
echo "Working dir: $ROOT_DIR"
echo "PROPTEST_CASES: $PROPTEST_CASES"

# 3) Mutation-style safety checks (tests should kill intentional bugs)
run ./mutation_guard.sh

# 4) Long-run adversarial fuzzing
run env PROPTEST_CASES="$PROPTEST_CASES" cargo test proptest_stateful_auction_invariants
run env PROPTEST_CASES="$PROPTEST_CASES" cargo test proptest_multi_auction_token_lock_invariants

# 5) Gas/DoS-style stress tests (high-cardinality loops/paths)
run cargo test --test integration integration_stress_many_bidders_and_withdraw_fanout
run cargo test --test integration integration_stress_large_escrow_and_swap_cycle

echo ""
echo "Security stress suite passed."
