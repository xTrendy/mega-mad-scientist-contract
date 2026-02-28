#!/usr/bin/env bash
# Mutation guard:
# Creates temporary mutated copies of the codebase and verifies
# targeted tests FAIL (mutants are "killed").

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
TMP_ROOT="$(mktemp -d)"
SURVIVORS=()

cleanup() {
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

copy_repo() {
  local dst="$1"
  mkdir -p "$dst"
  rsync -a \
    --exclude target \
    --exclude .git \
    --exclude logs \
    "$ROOT_DIR/" "$dst/"
}

apply_replace() {
  local file="$1"
  local from="$2"
  local to="$3"
  local mode="${4:-once}"
  python3 - "$file" "$from" "$to" "$mode" <<'PY'
import pathlib
import sys

file_path = pathlib.Path(sys.argv[1])
old = sys.argv[2]
new = sys.argv[3]
mode = sys.argv[4]
text = file_path.read_text()
count = text.count(old)
if count < 1:
    raise SystemExit(f"expected at least one match in {file_path}, found {count}")
if mode == "once":
    file_path.write_text(text.replace(old, new, 1))
elif mode == "all":
    file_path.write_text(text.replace(old, new))
else:
    raise SystemExit(f"unknown mode: {mode}")
PY
}

run_mutant() {
  local name="$1"
  local rel_file="$2"
  local from="$3"
  local to="$4"
  local test_cmd="$5"

  local mutant_dir="$TMP_ROOT/$name"
  copy_repo "$mutant_dir"
  apply_replace "$mutant_dir/$rel_file" "$from" "$to" "${6:-once}"

  echo ""
  echo "== Mutant: $name =="
  echo "Running: $test_cmd"

  if (cd "$mutant_dir" && bash -lc "$test_cmd" >/tmp/mutation_guard_${name}.log 2>&1); then
    echo "SURVIVED: $name (targeted test still passed)"
    SURVIVORS+=("$name")
  else
    echo "KILLED: $name"
  fi
}

echo "== Mutation Guard =="
echo "Root: $ROOT_DIR"

run_mutant \
  "tie_rule_weakened" \
  "src/contract.rs" \
  "total_bid_count > auction.highest_bid_count || auction.highest_bidder.is_none();" \
  "total_bid_count >= auction.highest_bid_count || auction.highest_bidder.is_none();" \
  "cargo test test_full_auction_to_pool_to_swap_flow"

run_mutant \
  "cross_auction_lock_disabled" \
  "src/contract.rs" \
  "if locked_auction_id != auction_id {" \
  "if false && locked_auction_id != auction_id {" \
  "cargo test test_same_token_cannot_be_reused_across_simultaneous_mega_auctions"

run_mutant \
  "no_winner_returns_to_admin" \
  "src/contract.rs" \
  "recipient: auction.depositor.to_string()," \
  "recipient: config.admin.to_string()," \
  "cargo test test_finalize_no_bids_returns_mega_to_original_depositor_after_admin_transfer" \
  "all"

run_mutant \
  "completed_withdraw_blocked" \
  "src/contract.rs" \
  "AuctionStatus::Finalizing | AuctionStatus::Cancelled | AuctionStatus::Completed => {}" \
  "AuctionStatus::Finalizing | AuctionStatus::Cancelled => {}" \
  "cargo test test_finalize_sub_minimum_bid_allows_withdraw_after_completed"

run_mutant \
  "funds_check_removed" \
  "src/contract.rs" \
  "if !info.funds.is_empty() {" \
  "if false && !info.funds.is_empty() {" \
  "cargo test --test integration integration_funds_rejected"

echo ""
if [ "${#SURVIVORS[@]}" -gt 0 ]; then
  echo "Mutation guard FAILED. Surviving mutants:"
  for m in "${SURVIVORS[@]}"; do
    echo " - $m"
  done
  exit 1
fi

echo "Mutation guard passed (all mutants killed)."
