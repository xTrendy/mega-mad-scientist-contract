#!/usr/bin/env bash
# Repeat-run regression harness for flake/nondeterminism detection.
# - Replays seeded determinism test many times and enforces stable hash per seed.
# - Repeats critical integration tests to catch intermittent failures.
# - Stores run logs and replay artifacts under logs/ci_repeat for CI upload.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

ITERATIONS="${ITERATIONS:-25}"
CRITICAL_REPEAT="${CRITICAL_REPEAT:-10}"
SEEDS="${SEEDS:-1 42 7777 20260228}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${ARTIFACT_DIR:-$ROOT_DIR/logs/ci_repeat/$STAMP}"
mkdir -p "$ARTIFACT_DIR"

SUMMARY_FILE="$ARTIFACT_DIR/summary.txt"
FAILURE_FILE="$ARTIFACT_DIR/FAILURE_SUMMARY.txt"

echo "== Repeat Regression Harness =="
echo "Working dir: $ROOT_DIR"
echo "ITERATIONS: $ITERATIONS"
echo "CRITICAL_REPEAT: $CRITICAL_REPEAT"
echo "SEEDS: $SEEDS"
echo "ARTIFACT_DIR: $ARTIFACT_DIR"
echo ""

{
  echo "Repeat Regression Harness"
  echo "stamp=$STAMP"
  echo "iterations=$ITERATIONS"
  echo "critical_repeat=$CRITICAL_REPEAT"
  echo "seeds=$SEEDS"
} >"$SUMMARY_FILE"

extract_hash() {
  local log_file="$1"
  if command -v rg >/dev/null 2>&1; then
    rg -o 'DETERMINISM_HASH=[0-9]+' "$log_file" | tail -n1 | cut -d= -f2
  else
    grep -Eo 'DETERMINISM_HASH=[0-9]+' "$log_file" | tail -n1 | cut -d= -f2
  fi
}

record_failure_and_exit() {
  local msg="$1"
  echo "$msg" | tee -a "$FAILURE_FILE"
  echo "failure=$msg" >>"$SUMMARY_FILE"
  exit 1
}

for seed in $SEEDS; do
  baseline_hash=""
  echo "Determinism seed $seed ($ITERATIONS runs)"
  echo "seed=$seed" >>"$SUMMARY_FILE"

  for run_id in $(seq 1 "$ITERATIONS"); do
    log_file="$ARTIFACT_DIR/determinism_seed_${seed}_run_${run_id}.log"
    if ! DETERMINISM_SEED="$seed" \
      DETERMINISM_RUN_ID="$run_id" \
      DETERMINISM_ARTIFACT_DIR="$ARTIFACT_DIR" \
      cargo test --test integration determinism_replay_emit_seed_snapshot -- --nocapture \
      >"$log_file" 2>&1; then
      record_failure_and_exit "determinism test failed: seed=$seed run=$run_id log=$log_file"
    fi

    hash="$(extract_hash "$log_file" || true)"
    if [[ -z "$hash" ]]; then
      record_failure_and_exit "missing DETERMINISM_HASH in log: seed=$seed run=$run_id log=$log_file"
    fi

    if [[ -z "$baseline_hash" ]]; then
      baseline_hash="$hash"
      echo "seed=$seed baseline_hash=$baseline_hash" >>"$SUMMARY_FILE"
    elif [[ "$hash" != "$baseline_hash" ]]; then
      echo "seed=$seed baseline_hash=$baseline_hash mismatch_hash=$hash run=$run_id" >>"$SUMMARY_FILE"
      record_failure_and_exit "hash mismatch for seed=$seed run=$run_id baseline=$baseline_hash got=$hash"
    fi
  done
done

critical_tests=(
  "integration_five_simultaneous_megas_no_double_dip"
  "integration_finalize_is_atomic_when_mega_transfer_fails"
  "integration_withdraw_is_atomic_when_mad_transfer_fails"
  "integration_anti_snipe_extends_auction"
  "integration_full_swap_flow"
)

echo ""
echo "Critical test repeats ($CRITICAL_REPEAT runs each)"
for test_name in "${critical_tests[@]}"; do
  echo "Test: $test_name"
  echo "test=$test_name" >>"$SUMMARY_FILE"
  for run_id in $(seq 1 "$CRITICAL_REPEAT"); do
    log_file="$ARTIFACT_DIR/critical_${test_name}_run_${run_id}.log"
    if ! cargo test --test integration "$test_name" >"$log_file" 2>&1; then
      record_failure_and_exit "critical test failed: test=$test_name run=$run_id log=$log_file"
    fi
  done
done

echo "status=ok" >>"$SUMMARY_FILE"
echo ""
echo "Repeat regression harness passed."
echo "Artifacts: $ARTIFACT_DIR"
