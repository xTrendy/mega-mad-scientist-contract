#!/usr/bin/env bash
# Real local-chain E2E harness for Cosmic Mad Scientist contract.
# Runs against a live local wasmd node and validates:
# - CW721 deployments (Standard + Cosmic)
# - Auction creation via ReceiveNft deposit_mega
# - Multi-bid flow and finalize
# - Loser withdraw
# - Swap deposit + claim

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

# Network / CLI config
CHAIN_ID="${CHAIN_ID:-localwasm}"
NODE="${NODE:-http://127.0.0.1:26657}"
WASMD_HOME="${WASMD_HOME:-$HOME/.wasmd}"
KEYRING_BACKEND="${KEYRING_BACKEND:-test}"
DENOM="${DENOM:-stake}"
GAS_PRICES="${GAS_PRICES:-0.025${DENOM}}"
GAS_ADJUSTMENT="${GAS_ADJUSTMENT:-1.4}"

# Keys
ADMIN_KEY="${ADMIN_KEY:-admin}"
BIDDER1_KEY="${BIDDER1_KEY:-bidder1}"
BIDDER2_KEY="${BIDDER2_KEY:-bidder2}"
FUNDER_KEY="${FUNDER_KEY:-validator}"
AUTO_FUND="${AUTO_FUND:-true}"

# Artifacts
AUCTION_WASM="${AUCTION_WASM:-target/wasm32-unknown-unknown/release/mega_mad_scientist.wasm}"
CW721_WASM="${CW721_WASM:-artifacts/cw721_base.wasm}"
FORCE_AUCTION_REBUILD="${FORCE_AUCTION_REBUILD:-true}"
WASM_RUST_TOOLCHAIN="${WASM_RUST_TOOLCHAIN:-stable}"
WASM_RUSTFLAGS_COMPAT="${WASM_RUSTFLAGS_COMPAT:--C target-cpu=mvp -C target-feature=-bulk-memory,-reference-types,-multivalue,-sign-ext -C link-arg=-s}"

# Runtime
WAIT_POLL_SECONDS="${WAIT_POLL_SECONDS:-2}"
WAIT_MAX_POLLS="${WAIT_MAX_POLLS:-45}"
MIN_REQUIRED_BALANCE="${MIN_REQUIRED_BALANCE:-20000000}"

TX_COMMON=(
  --node "$NODE"
  --chain-id "$CHAIN_ID"
  --home "$WASMD_HOME"
  --keyring-backend "$KEYRING_BACKEND"
  --gas auto
  --gas-adjustment "$GAS_ADJUSTMENT"
  --gas-prices "$GAS_PRICES"
  --broadcast-mode sync
  --output json
  -y
)

log() {
  echo "[localnet-e2e] $*"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $1" >&2
    exit 1
  fi
}

ensure_edition2024_capable_toolchain() {
  local cargo_version
  local major
  local minor

  cargo_version="$(cargo +"${WASM_RUST_TOOLCHAIN}" --version 2>/dev/null | awk '{print $2}' || true)"
  if [[ ! "$cargo_version" =~ ^[0-9]+\.[0-9]+ ]]; then
    echo "ERROR: could not determine cargo version for toolchain '${WASM_RUST_TOOLCHAIN}'." >&2
    exit 1
  fi

  major="${cargo_version%%.*}"
  minor="${cargo_version#*.}"
  minor="${minor%%.*}"

  if (( major < 1 || (major == 1 && minor < 85) )); then
    echo "ERROR: toolchain '${WASM_RUST_TOOLCHAIN}' provides cargo ${cargo_version}, but dependencies require edition2024 support (cargo >= 1.85)." >&2
    echo "Set WASM_RUST_TOOLCHAIN=stable (recommended) or any toolchain >= 1.85 and retry." >&2
    exit 1
  fi
}

assert_eq() {
  local actual="$1"
  local expected="$2"
  local context="$3"
  if [[ "$actual" != "$expected" ]]; then
    echo "ASSERT FAILED ($context): expected '$expected', got '$actual'" >&2
    exit 1
  fi
  log "assert ok: $context = $expected"
}

b64() {
  printf '%s' "$1" | base64 | tr -d '\n'
}

build_auction_wasm_compat() {
  log "building auction wasm with compatibility toolchain ${WASM_RUST_TOOLCHAIN}"
  rustup toolchain install "${WASM_RUST_TOOLCHAIN}" >/dev/null
  ensure_edition2024_capable_toolchain
  rustup target add wasm32-unknown-unknown --toolchain "${WASM_RUST_TOOLCHAIN}" >/dev/null
  CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
  CARGO_PROFILE_RELEASE_LTO=true \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
  CARGO_PROFILE_RELEASE_PANIC=abort \
  RUSTFLAGS="${WASM_RUSTFLAGS_COMPAT}" \
  cargo +"${WASM_RUST_TOOLCHAIN}" build --release --target wasm32-unknown-unknown
}

ensure_key() {
  local key="$1"
  if ! wasmd keys show "$key" --home "$WASMD_HOME" --keyring-backend "$KEYRING_BACKEND" >/dev/null 2>&1; then
    log "creating missing key: $key"
    wasmd keys add "$key" --home "$WASMD_HOME" --keyring-backend "$KEYRING_BACKEND" >/dev/null 2>&1
  fi
}

addr_of() {
  local key="$1"
  wasmd keys show "$key" -a --home "$WASMD_HOME" --keyring-backend "$KEYRING_BACKEND"
}

balance_of() {
  local addr="$1"
  wasmd query bank balances "$addr" --node "$NODE" --output json \
    | jq -r --arg denom "$DENOM" '.balances[]? | select(.denom == $denom) | .amount' \
    | head -n1
}

wait_for_tx() {
  local txhash="$1"
  local i=0
  local tx_json=""

  while (( i < WAIT_MAX_POLLS )); do
    i=$((i + 1))
    tx_json="$(wasmd query tx "$txhash" --node "$NODE" --output json 2>/dev/null || true)"
    if [[ -n "$tx_json" ]] && [[ "$(echo "$tx_json" | jq -r '.txhash // empty')" == "$txhash" ]]; then
      local code
      code="$(echo "$tx_json" | jq -r '.code // 0')"
      if [[ "$code" != "0" ]]; then
        echo "TX failed ($txhash) with code $code" >&2
        echo "$tx_json" | jq .
        exit 1
      fi
      echo "$tx_json"
      return 0
    fi
    sleep "$WAIT_POLL_SECONDS"
  done

  echo "Timed out waiting for tx: $txhash" >&2
  exit 1
}

run_tx() {
  local out
  local txhash

  out="$(wasmd tx "$@" "${TX_COMMON[@]}")"
  txhash="$(echo "$out" | jq -r '.txhash // empty')"

  if [[ -z "$txhash" ]]; then
    echo "No txhash returned for tx: wasmd tx $*" >&2
    echo "$out" >&2
    exit 1
  fi

  wait_for_tx "$txhash"
}

# Like run_tx but expects the transaction to FAIL (non-zero code).
# Returns the raw_log text so callers can check the error message.
run_tx_expect_fail() {
  local out
  local txhash
  local tx_json
  local code
  local raw_log

  out="$(wasmd tx "$@" "${TX_COMMON[@]}" 2>&1 || true)"
  txhash="$(echo "$out" | jq -r '.txhash // empty' 2>/dev/null || true)"

  if [[ -z "$txhash" ]]; then
    # Broadcast itself might have failed — check for error in output
    if echo "$out" | grep -qi "error\|failed\|invalid"; then
      log "tx rejected at broadcast (expected): ${*:1:4}..."
      echo "$out"
      return 0
    fi
    echo "No txhash and no error for expected-fail tx: wasmd tx $*" >&2
    echo "$out" >&2
    exit 1
  fi

  # Wait for tx to land
  local i=0
  while (( i < WAIT_MAX_POLLS )); do
    i=$((i + 1))
    tx_json="$(wasmd query tx "$txhash" --node "$NODE" --output json 2>/dev/null || true)"
    if [[ -n "$tx_json" ]] && [[ "$(echo "$tx_json" | jq -r '.txhash // empty')" == "$txhash" ]]; then
      code="$(echo "$tx_json" | jq -r '.code // 0')"
      raw_log="$(echo "$tx_json" | jq -r '.raw_log // empty')"
      if [[ "$code" == "0" ]]; then
        echo "ASSERT FAILED: expected tx to fail but it succeeded: wasmd tx $*" >&2
        echo "$tx_json" | jq . >&2
        exit 1
      fi
      log "tx failed as expected (code=$code): ${*:1:4}..."
      echo "$raw_log"
      return 0
    fi
    sleep "$WAIT_POLL_SECONDS"
  done

  echo "Timed out waiting for expected-fail tx: $txhash" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local context="$3"
  if echo "$haystack" | grep -qi "$needle"; then
    log "assert ok: $context contains '$needle'"
  else
    echo "ASSERT FAILED ($context): expected to contain '$needle' in:" >&2
    echo "$haystack" >&2
    exit 1
  fi
}

query_contract_data() {
  local contract="$1"
  local msg="$2"
  wasmd query wasm contract-state smart "$contract" "$msg" --node "$NODE" --output json | jq -c '.data // .'
}

extract_last_event_value() {
  local tx_json="$1"
  local event_type="$2"
  local key="$3"
  echo "$tx_json" | jq -r \
    --arg event_type "$event_type" \
    --arg key "$key" \
    '[.events[]? | select(.type == $event_type) | .attributes[]? | select(.key == $key) | .value][-1] // empty'
}

store_code() {
  local wasm_path="$1"
  local tx_json
  local code_id

  tx_json="$(run_tx wasm store "$wasm_path" --from "$ADMIN_KEY")"
  code_id="$(extract_last_event_value "$tx_json" "store_code" "code_id")"

  if [[ -z "$code_id" ]]; then
    echo "Failed to extract code_id after storing $wasm_path" >&2
    echo "$tx_json" | jq .
    exit 1
  fi

  echo "$code_id"
}

instantiate_code() {
  local code_id="$1"
  local msg="$2"
  local label="$3"
  local tx_json
  local contract_addr

  tx_json="$(run_tx wasm instantiate "$code_id" "$msg" --label "$label" --admin "$ADMIN_ADDR" --from "$ADMIN_KEY")"
  contract_addr="$(
    echo "$tx_json" | jq -r \
      '[.events[]? | select(.type == "instantiate" or .type == "instantiate_contract")
        | .attributes[]? | select(.key == "_contract_address" or .key == "contract_address")
        | .value][-1] // empty'
  )"

  if [[ -z "$contract_addr" ]]; then
    echo "Failed to extract contract address for instantiate label: $label" >&2
    echo "$tx_json" | jq .
    exit 1
  fi

  echo "$contract_addr"
}

log "checking prerequisites"
require_cmd wasmd
require_cmd jq
require_cmd base64

if ! wasmd status --node "$NODE" >/dev/null 2>&1; then
  echo "ERROR: cannot reach local wasmd node at $NODE" >&2
  echo "Start your local chain first, then rerun this script." >&2
  exit 1
fi

if [[ "$AUCTION_WASM" == "target/wasm32-unknown-unknown/release/mega_mad_scientist.wasm" ]] && [[ "$FORCE_AUCTION_REBUILD" == "true" ]]; then
  build_auction_wasm_compat
elif [[ ! -f "$AUCTION_WASM" ]]; then
  log "auction wasm missing at $AUCTION_WASM"
  if [[ "$AUCTION_WASM" == "target/wasm32-unknown-unknown/release/mega_mad_scientist.wasm" ]]; then
    build_auction_wasm_compat
  else
    echo "Set AUCTION_WASM to an existing compatible wasm artifact and retry." >&2
    exit 1
  fi
fi

if [[ ! -f "$CW721_WASM" ]]; then
  echo "ERROR: CW721 wasm not found at $CW721_WASM" >&2
  echo "Set CW721_WASM to a compiled cw721-base contract wasm path." >&2
  exit 1
fi

ensure_key "$ADMIN_KEY"
ensure_key "$BIDDER1_KEY"
ensure_key "$BIDDER2_KEY"

ADMIN_ADDR="$(addr_of "$ADMIN_KEY")"
BIDDER1_ADDR="$(addr_of "$BIDDER1_KEY")"
BIDDER2_ADDR="$(addr_of "$BIDDER2_KEY")"

if [[ "$AUTO_FUND" == "true" ]] && wasmd keys show "$FUNDER_KEY" --home "$WASMD_HOME" --keyring-backend "$KEYRING_BACKEND" >/dev/null 2>&1; then
  FUNDER_ADDR="$(addr_of "$FUNDER_KEY")"
  for target in "$ADMIN_ADDR" "$BIDDER1_ADDR" "$BIDDER2_ADDR"; do
    bal="$(balance_of "$target")"
    bal="${bal:-0}"
    if [[ "$bal" =~ ^[0-9]+$ ]] && (( bal < MIN_REQUIRED_BALANCE )) && [[ "$target" != "$FUNDER_ADDR" ]]; then
      log "auto-funding $target from $FUNDER_KEY"
      run_tx bank send "$FUNDER_ADDR" "$target" "50000000${DENOM}" --from "$FUNDER_KEY" >/dev/null
    fi
  done
fi

for target in "$ADMIN_ADDR" "$BIDDER1_ADDR" "$BIDDER2_ADDR"; do
  bal="$(balance_of "$target")"
  bal="${bal:-0}"
  if ! [[ "$bal" =~ ^[0-9]+$ ]] || (( bal < 1 )); then
    echo "ERROR: account has no ${DENOM} balance: $target" >&2
    echo "Fund it or configure AUTO_FUND/FUNDER_KEY, then retry." >&2
    exit 1
  fi
done

log "storing contracts"
CW721_CODE_ID="$(store_code "$CW721_WASM")"
AUCTION_CODE_ID="$(store_code "$AUCTION_WASM")"
log "cw721 code id: $CW721_CODE_ID"
log "auction code id: $AUCTION_CODE_ID"

log "instantiating Standard and Cosmic CW721 collections"
MAD_INIT_MSG="$(jq -cn --arg minter "$ADMIN_ADDR" '{name:"Standard Mad Scientists",symbol:"MAD",collection_info_extension:null,minter:$minter,creator:null,withdraw_address:null}')"
COSMIC_INIT_MSG="$(jq -cn --arg minter "$ADMIN_ADDR" '{name:"Cosmic Mad Scientists",symbol:"COSMIC",collection_info_extension:null,minter:$minter,creator:null,withdraw_address:null}')"

MAD_ADDR="$(instantiate_code "$CW721_CODE_ID" "$MAD_INIT_MSG" "mad-standard")"
COSMIC_ADDR="$(instantiate_code "$CW721_CODE_ID" "$COSMIC_INIT_MSG" "mad-cosmic")"
log "mad addr: $MAD_ADDR"
log "cosmic addr: $COSMIC_ADDR"

log "instantiating auction contract"
AUCTION_INIT_MSG="$(jq -cn \
  --arg admin "$ADMIN_ADDR" \
  --arg mad "$MAD_ADDR" \
  --arg cosmic "$COSMIC_ADDR" \
  '{admin:$admin,mad_scientist_collection:$mad,mega_mad_scientist_collection:$cosmic,default_min_bid:1,anti_snipe_window:5,anti_snipe_extension:5,max_extension:60,max_bidders_per_auction:100,max_staging_size:50,max_nfts_per_bid:50}')"

AUCTION_ADDR="$(instantiate_code "$AUCTION_CODE_ID" "$AUCTION_INIT_MSG" "cosmic-auction")"
log "auction addr: $AUCTION_ADDR"

log "minting NFTs"
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-1","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-1","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-2","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-3","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-4","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-5","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

start_time=$(( $(date +%s) - 15 ))
end_time=$(( $(date +%s) + 45 ))

log "creating auction via CW721 SendNft (deposit_mega)"
deposit_hook="$(jq -cn --argjson start "$start_time" --argjson end "$end_time" '{"deposit_mega":{"start_time":$start,"end_time":$end,"min_bid":1}}')"
deposit_b64="$(b64 "$deposit_hook")"
run_tx wasm execute "$COSMIC_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "cosmic-1" --arg msg "$deposit_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$ADMIN_KEY" >/dev/null

bid_hook_b64="$(b64 '{"bid":{"auction_id":1}}')"

log "placing bids"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-1" --arg msg "$bid_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-2" --arg msg "$bid_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-3" --arg msg "$bid_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-4" --arg msg "$bid_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-5" --arg msg "$bid_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null

auction_before_finalize="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":1}}')"
auction_end_time="$(echo "$auction_before_finalize" | jq -r '.auction.end_time')"
if [[ "$auction_end_time" =~ ^[0-9]+$ ]]; then
  now="$(date +%s)"
  if (( now <= auction_end_time )); then
    sleep_secs=$((auction_end_time - now + 20))
  else
    sleep_secs=5
  fi
  log "waiting ${sleep_secs}s before finalize (auction end buffer)"
  sleep "$sleep_secs"
else
  log "could not parse auction end time; waiting 35s fallback"
  sleep 35
fi

log "finalizing auction"
run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":1}}' --from "$ADMIN_KEY" >/dev/null

auction_data="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":1}}')"
highest_bidder="$(echo "$auction_data" | jq -r '.auction.highest_bidder // empty')"
status="$(echo "$auction_data" | jq -r '.auction.status // empty')"
assert_eq "$highest_bidder" "$BIDDER2_ADDR" "winner"
assert_eq "$status" "finalizing" "auction status after finalize"

pool_data="$(query_contract_data "$AUCTION_ADDR" '{"get_pool_size":{}}')"
pool_size="$(echo "$pool_data" | jq -r '.size')"
assert_eq "$pool_size" "3" "pool size after finalize"

# P1: Cosmic winner ownership assertion
log "P1: asserting cosmic-1 transferred to winner"
cosmic1_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-1","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic1_owner" "$BIDDER2_ADDR" "cosmic-1 owner after finalize"

# P1: Pool token-ID membership assertion
log "P1: asserting pool contains exactly the winner's bid tokens"
pool_contents="$(query_contract_data "$AUCTION_ADDR" '{"get_pool_contents":{}}')"
pool_tokens="$(echo "$pool_contents" | jq -r '[.token_ids[]] | sort | join(",")')"
assert_eq "$pool_tokens" "mad-3,mad-4,mad-5" "pool token IDs after finalize"

log "withdrawing losing bid for bidder1"
run_tx wasm execute "$AUCTION_ADDR" '{"withdraw_bid":{"auction_id":1}}' --from "$BIDDER1_KEY" >/dev/null

owner_mad1="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-1","include_expired":null}}' | jq -r '.owner')"
owner_mad2="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-2","include_expired":null}}' | jq -r '.owner')"
assert_eq "$owner_mad1" "$BIDDER1_ADDR" "owner mad-1 after withdraw"
assert_eq "$owner_mad2" "$BIDDER1_ADDR" "owner mad-2 after withdraw"

log "swap path: deposit 2 staged NFTs then claim 2 from pool"
swap_hook_b64="$(b64 '"swap_deposit"')"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-1" --arg msg "$swap_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-2" --arg msg "$swap_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null

run_tx wasm execute "$AUCTION_ADDR" '{"claim_swap":{"requested_ids":["mad-3","mad-4"]}}' --from "$BIDDER1_KEY" >/dev/null

owner_mad3="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-3","include_expired":null}}' | jq -r '.owner')"
owner_mad4="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-4","include_expired":null}}' | jq -r '.owner')"
assert_eq "$owner_mad3" "$BIDDER1_ADDR" "owner mad-3 after swap"
assert_eq "$owner_mad4" "$BIDDER1_ADDR" "owner mad-4 after swap"

log "happy path passed — beginning non-happy-path scenarios"

###############################################################################
# P2a: No-bid finalize — Cosmic returned to depositor
###############################################################################
log "P2a: no-bid finalize scenario"
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-2","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

nobid_start=$(( $(date +%s) - 30 ))
nobid_end=$(( $(date +%s) - 5 ))
nobid_hook="$(jq -cn --argjson start "$nobid_start" --argjson end "$nobid_end" '{"deposit_mega":{"start_time":$start,"end_time":$end,"min_bid":1}}')"
nobid_b64="$(b64 "$nobid_hook")"
run_tx wasm execute "$COSMIC_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "cosmic-2" --arg msg "$nobid_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$ADMIN_KEY" >/dev/null

log "P2a: finalizing no-bid auction"
run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":2}}' --from "$ADMIN_KEY" >/dev/null

nobid_data="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":2}}')"
nobid_status="$(echo "$nobid_data" | jq -r '.auction.status // empty')"
assert_eq "$nobid_status" "completed" "no-bid auction status"

cosmic2_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-2","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic2_owner" "$ADMIN_ADDR" "cosmic-2 returned to depositor after no-bid finalize"

###############################################################################
# P2b: Cancel before bids — Cosmic returned to depositor
###############################################################################
log "P2b: cancel-before-bids scenario"
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-3","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

cancel_start=$(( $(date +%s) - 10 ))
cancel_end=$(( $(date +%s) + 600 ))
cancel_hook="$(jq -cn --argjson start "$cancel_start" --argjson end "$cancel_end" '{"deposit_mega":{"start_time":$start,"end_time":$end,"min_bid":1}}')"
cancel_b64="$(b64 "$cancel_hook")"
run_tx wasm execute "$COSMIC_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "cosmic-3" --arg msg "$cancel_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$ADMIN_KEY" >/dev/null

run_tx wasm execute "$AUCTION_ADDR" '{"cancel_auction":{"auction_id":3}}' --from "$ADMIN_KEY" >/dev/null

cancel_data="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":3}}')"
cancel_status="$(echo "$cancel_data" | jq -r '.auction.status // empty')"
assert_eq "$cancel_status" "cancelled" "cancel-before-bids auction status"

cosmic3_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-3","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic3_owner" "$ADMIN_ADDR" "cosmic-3 returned after cancel"

###############################################################################
# P2c: Cancel after bids fails
###############################################################################
log "P2c: cancel-after-bids must fail"
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-4","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-6","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

cancel2_start=$(( $(date +%s) - 10 ))
cancel2_end=$(( $(date +%s) + 75 ))
cancel2_hook="$(jq -cn --argjson start "$cancel2_start" --argjson end "$cancel2_end" '{"deposit_mega":{"start_time":$start,"end_time":$end,"min_bid":1}}')"
cancel2_b64="$(b64 "$cancel2_hook")"
run_tx wasm execute "$COSMIC_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "cosmic-4" --arg msg "$cancel2_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$ADMIN_KEY" >/dev/null

bid4_hook_b64="$(b64 '{"bid":{"auction_id":4}}')"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-6" --arg msg "$bid4_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null

cancel_fail_log="$(run_tx_expect_fail wasm execute "$AUCTION_ADDR" '{"cancel_auction":{"auction_id":4}}' --from "$ADMIN_KEY")"
assert_contains "$cancel_fail_log" "bids have already been placed" "cancel-after-bids error"

###############################################################################
# P2d: Pause / unpause
###############################################################################
log "P2d: pause and unpause"
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-7","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

run_tx wasm execute "$AUCTION_ADDR" '{"set_paused":{"paused":true}}' --from "$ADMIN_KEY" >/dev/null

paused_config="$(query_contract_data "$AUCTION_ADDR" '{"get_config":{}}')"
paused_flag="$(echo "$paused_config" | jq -r '.paused')"
assert_eq "$paused_flag" "true" "contract paused flag"

bid_pause_hook_b64="$(b64 '{"bid":{"auction_id":4}}')"
pause_fail_log="$(run_tx_expect_fail wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-7" --arg msg "$bid_pause_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY")"
assert_contains "$pause_fail_log" "paused" "bid-while-paused error"

run_tx wasm execute "$AUCTION_ADDR" '{"set_paused":{"paused":false}}' --from "$ADMIN_KEY" >/dev/null

log "P2d: placing bid after unpause"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-7" --arg msg "$bid_pause_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
log "assert ok: bid succeeded after unpause"

###############################################################################
# P2e: Force-complete auction
###############################################################################
log "P2e: force-complete auction 4"
# Auction 4 has a single bidder (bidder1), so bidder1 is the winner.
# Winner bid NFTs go to pool and are not withdrawable as losing escrow.
auction4_data="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":4}}')"
auction4_end="$(echo "$auction4_data" | jq -r '.auction.end_time')"
now="$(date +%s)"
if [[ "$auction4_end" =~ ^[0-9]+$ ]] && (( now <= auction4_end )); then
  fc_wait=$((auction4_end - now + 10))
  log "waiting ${fc_wait}s for auction 4 to end"
  sleep "$fc_wait"
fi

run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":4}}' --from "$ADMIN_KEY" >/dev/null

fc_status="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":4}}' | jq -r '.auction.status')"
assert_eq "$fc_status" "finalizing" "auction 4 status after finalize"

run_tx wasm execute "$AUCTION_ADDR" '{"force_complete_auction":{"auction_id":4}}' --from "$ADMIN_KEY" >/dev/null

fc_status2="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":4}}' | jq -r '.auction.status')"
assert_eq "$fc_status2" "completed" "auction 4 status after force-complete"

# Winner withdraw must fail (no losing escrow for bidder1 on auction 4)
winner_withdraw_log="$(run_tx_expect_fail wasm execute "$AUCTION_ADDR" '{"withdraw_bid":{"auction_id":4}}' --from "$BIDDER1_KEY")"
assert_contains "$winner_withdraw_log" "No escrowed NFTs found" "winner cannot withdraw bid escrow after force-complete"

# Cosmic-4 should go to winner (bidder1 is only bidder)
cosmic4_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-4","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic4_owner" "$BIDDER1_ADDR" "cosmic-4 transferred to winner after finalize"

###############################################################################
# P2f: Withdraw-staged
###############################################################################
log "P2f: withdraw-staged scenario"
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-8","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-9","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

swap_hook_b64="$(b64 '"swap_deposit"')"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-8" --arg msg "$swap_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-9" --arg msg "$swap_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null

staging_data="$(query_contract_data "$AUCTION_ADDR" "$(jq -cn --arg user "$BIDDER1_ADDR" '{"get_swap_staging":{"user":$user}}')")"
staged_count="$(echo "$staging_data" | jq -r '[.token_ids[]] | length')"
assert_eq "$staged_count" "2" "staged token count before withdraw"

run_tx wasm execute "$AUCTION_ADDR" '{"withdraw_staged":{}}' --from "$BIDDER1_KEY" >/dev/null

owner_mad8="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-8","include_expired":null}}' | jq -r '.owner')"
owner_mad9="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-9","include_expired":null}}' | jq -r '.owner')"
assert_eq "$owner_mad8" "$BIDDER1_ADDR" "mad-8 returned after withdraw-staged"
assert_eq "$owner_mad9" "$BIDDER1_ADDR" "mad-9 returned after withdraw-staged"

staging_after="$(query_contract_data "$AUCTION_ADDR" "$(jq -cn --arg user "$BIDDER1_ADDR" '{"get_swap_staging":{"user":$user}}')")"
staged_after="$(echo "$staging_after" | jq -r '[.token_ids[]] | length')"
assert_eq "$staged_after" "0" "staged tokens after withdraw-staged"

###############################################################################
# P2g: Funds rejection
###############################################################################
log "P2g: funds rejection"
funds_fail_log="$(run_tx_expect_fail wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":1}}' --from "$ADMIN_KEY" --amount "1${DENOM}")"
assert_contains "$funds_fail_log" "does not accept funds" "funds rejection error"

###############################################################################
# P2h: Admin transfer (propose + accept)
###############################################################################
log "P2h: admin transfer"
run_tx wasm execute "$AUCTION_ADDR" "$(jq -cn --arg new "$BIDDER1_ADDR" '{"propose_admin":{"new_admin":$new}}')" --from "$ADMIN_KEY" >/dev/null

pending_config="$(query_contract_data "$AUCTION_ADDR" '{"get_config":{}}')"
pending_admin="$(echo "$pending_config" | jq -r '.pending_admin // empty')"
assert_eq "$pending_admin" "$BIDDER1_ADDR" "pending admin after propose"

run_tx wasm execute "$AUCTION_ADDR" '{"accept_admin":{}}' --from "$BIDDER1_KEY" >/dev/null

new_config="$(query_contract_data "$AUCTION_ADDR" '{"get_config":{}}')"
new_admin="$(echo "$new_config" | jq -r '.admin')"
assert_eq "$new_admin" "$BIDDER1_ADDR" "admin after accept"

# Transfer back to original admin
run_tx wasm execute "$AUCTION_ADDR" "$(jq -cn --arg new "$ADMIN_ADDR" '{"propose_admin":{"new_admin":$new}}')" --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$AUCTION_ADDR" '{"accept_admin":{}}' --from "$ADMIN_KEY" >/dev/null

restored_admin="$(query_contract_data "$AUCTION_ADDR" '{"get_config":{}}' | jq -r '.admin')"
assert_eq "$restored_admin" "$ADMIN_ADDR" "admin restored after round-trip transfer"

log "Priority 2 non-happy-path scenarios passed"

###############################################################################
# P3: Multi-auction with cross-auction double-dip prevention
###############################################################################
log "P3: multi-auction cross-auction double-dip scenario"

# Mint Cosmic NFTs for 3 simultaneous auctions
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-5","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-6","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-7","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

# Mint Standard NFTs for multi-auction bids
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"multi-1","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"multi-2","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"multi-3","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"multi-4","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"multi-5","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"multi-6","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

# Create 3 overlapping auctions (5, 6, 7)
multi_start=$(( $(date +%s) - 10 ))
multi_end=$(( $(date +%s) + 60 ))

for cosmic_id in cosmic-5 cosmic-6 cosmic-7; do
  multi_hook="$(jq -cn --argjson start "$multi_start" --argjson end "$multi_end" '{"deposit_mega":{"start_time":$start,"end_time":$end,"min_bid":1}}')"
  multi_b64="$(b64 "$multi_hook")"
  run_tx wasm execute "$COSMIC_ADDR" \
    "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "$cosmic_id" --arg msg "$multi_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
    --from "$ADMIN_KEY" >/dev/null
done

log "P3: bidding on auction 5"
bid5_hook_b64="$(b64 '{"bid":{"auction_id":5}}')"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "multi-1" --arg msg "$bid5_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "multi-2" --arg msg "$bid5_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null

log "P3: attempting cross-auction double-dip (multi-1 already in auction 5 → auction 6)"
bid6_hook_b64="$(b64 '{"bid":{"auction_id":6}}')"
# multi-1 is escrowed in auction 5 — the contract holds it, so bidder1 can't send it again.
# The CW721 contract itself should reject this since bidder1 no longer owns multi-1.
doubledip_log="$(run_tx_expect_fail wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "multi-1" --arg msg "$bid6_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY")"
log "assert ok: cross-auction double-dip rejected"

log "P3: bidding on auction 6 with different tokens"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "multi-4" --arg msg "$bid6_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "multi-5" --arg msg "$bid6_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "multi-6" --arg msg "$bid6_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null

# Wait for all multi-auctions to end
auction5_data="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":5}}')"
auction5_end="$(echo "$auction5_data" | jq -r '.auction.end_time')"
now="$(date +%s)"
if [[ "$auction5_end" =~ ^[0-9]+$ ]] && (( now <= auction5_end )); then
  multi_wait=$((auction5_end - now + 10))
  log "waiting ${multi_wait}s for multi-auctions to end"
  sleep "$multi_wait"
fi

log "P3: finalizing auctions 5, 6, 7"
run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":5}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":6}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":7}}' --from "$ADMIN_KEY" >/dev/null

# Auction 5: bidder1 is the only bidder → winner
a5_winner="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":5}}' | jq -r '.auction.highest_bidder')"
assert_eq "$a5_winner" "$BIDDER1_ADDR" "auction 5 winner"
cosmic5_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-5","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic5_owner" "$BIDDER1_ADDR" "cosmic-5 transferred to auction 5 winner"

# Auction 6: bidder2 is the only bidder → winner
a6_winner="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":6}}' | jq -r '.auction.highest_bidder')"
assert_eq "$a6_winner" "$BIDDER2_ADDR" "auction 6 winner"
cosmic6_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-6","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic6_owner" "$BIDDER2_ADDR" "cosmic-6 transferred to auction 6 winner"

# Auction 7: no bids → cosmic-7 returned to admin
a7_status="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":7}}' | jq -r '.auction.status')"
assert_eq "$a7_status" "completed" "auction 7 no-bid status"
cosmic7_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-7","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic7_owner" "$ADMIN_ADDR" "cosmic-7 returned after no-bid finalize"

# Pool size should include all winner bid escrow that moved to pool:
# After auction 1 swap flow: 3
# +2 from auction 4 winner bid (mad-6,mad-7) = 5
# +2 from auction 5 winner bid (multi-1,multi-2) = 7
# +3 from auction 6 winner bid (multi-4,multi-5,multi-6) = 10
final_pool_size="$(query_contract_data "$AUCTION_ADDR" '{"get_pool_size":{}}' | jq -r '.size')"
assert_eq "$final_pool_size" "10" "final pool size after all auctions"

log "Priority 3 multi-auction scenarios passed"

###############################################################################
# P3b: Multi-bidder force-complete with loser withdraw after completion
###############################################################################
log "P3b: multi-bidder force-complete + loser withdraw scenario"

run_tx wasm execute "$COSMIC_ADDR" '{"mint":{"token_id":"cosmic-8","owner":"'"$ADMIN_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-10","owner":"'"$BIDDER1_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-11","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" '{"mint":{"token_id":"mad-12","owner":"'"$BIDDER2_ADDR"'","token_uri":null,"extension":null}}' --from "$ADMIN_KEY" >/dev/null

fc2_start=$(( $(date +%s) - 10 ))
fc2_end=$(( $(date +%s) + 35 ))
fc2_hook="$(jq -cn --argjson start "$fc2_start" --argjson end "$fc2_end" '{"deposit_mega":{"start_time":$start,"end_time":$end,"min_bid":1}}')"
fc2_b64="$(b64 "$fc2_hook")"
run_tx wasm execute "$COSMIC_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "cosmic-8" --arg msg "$fc2_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$ADMIN_KEY" >/dev/null

bid8_hook_b64="$(b64 '{"bid":{"auction_id":8}}')"
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-10" --arg msg "$bid8_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER1_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-11" --arg msg "$bid8_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null
run_tx wasm execute "$MAD_ADDR" \
  "$(jq -cn --arg contract "$AUCTION_ADDR" --arg token "mad-12" --arg msg "$bid8_hook_b64" '{"send_nft":{"contract":$contract,"token_id":$token,"msg":$msg}}')" \
  --from "$BIDDER2_KEY" >/dev/null

auction8_data="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":8}}')"
auction8_end="$(echo "$auction8_data" | jq -r '.auction.end_time')"
now="$(date +%s)"
if [[ "$auction8_end" =~ ^[0-9]+$ ]] && (( now <= auction8_end )); then
  fc2_wait=$((auction8_end - now + 10))
  log "waiting ${fc2_wait}s for auction 8 to end"
  sleep "$fc2_wait"
fi

run_tx wasm execute "$AUCTION_ADDR" '{"finalize_auction":{"auction_id":8}}' --from "$ADMIN_KEY" >/dev/null
a8_status="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":8}}' | jq -r '.auction.status')"
assert_eq "$a8_status" "finalizing" "auction 8 status after finalize"

run_tx wasm execute "$AUCTION_ADDR" '{"force_complete_auction":{"auction_id":8}}' --from "$ADMIN_KEY" >/dev/null
a8_status2="$(query_contract_data "$AUCTION_ADDR" '{"get_auction":{"auction_id":8}}' | jq -r '.auction.status')"
assert_eq "$a8_status2" "completed" "auction 8 status after force-complete"

run_tx wasm execute "$AUCTION_ADDR" '{"withdraw_bid":{"auction_id":8}}' --from "$BIDDER1_KEY" >/dev/null
owner_mad10="$(query_contract_data "$MAD_ADDR" '{"owner_of":{"token_id":"mad-10","include_expired":null}}' | jq -r '.owner')"
assert_eq "$owner_mad10" "$BIDDER1_ADDR" "mad-10 returned to losing bidder after force-complete"

cosmic8_owner="$(query_contract_data "$COSMIC_ADDR" '{"owner_of":{"token_id":"cosmic-8","include_expired":null}}' | jq -r '.owner')"
assert_eq "$cosmic8_owner" "$BIDDER2_ADDR" "cosmic-8 transferred to winner in force-complete scenario"

log "P3b multi-bidder force-complete loser-withdraw scenario passed"

log "========================================="
log "FULL E2E local-chain test suite passed."
log "========================================="
log "ADMIN=$ADMIN_ADDR"
log "BIDDER1=$BIDDER1_ADDR"
log "BIDDER2=$BIDDER2_ADDR"
log "MAD=$MAD_ADDR"
log "COSMIC=$COSMIC_ADDR"
log "AUCTION=$AUCTION_ADDR"
