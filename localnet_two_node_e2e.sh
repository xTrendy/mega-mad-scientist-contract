#!/usr/bin/env bash
# One-command local-only test harness:
# 1) boot a 2-node local wasmd network
# 2) run localnet_e2e.sh against node0 RPC
# 3) stop nodes (unless KEEP_RUNNING=true)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

CHAIN_ID="${CHAIN_ID:-localwasm}"
DENOM="${DENOM:-stake}"
KEYRING_BACKEND="${KEYRING_BACKEND:-test}"

LOCALNET_DIR="${LOCALNET_DIR:-$ROOT_DIR/.localnet/two-node}"
LOG_DIR="$LOCALNET_DIR/logs"
NODE0_HOME="$LOCALNET_DIR/node0"
NODE1_HOME="$LOCALNET_DIR/node1"

RESET_LOCALNET="${RESET_LOCALNET:-true}"
KEEP_RUNNING="${KEEP_RUNNING:-false}"
AUTO_BUILD_CW721="${AUTO_BUILD_CW721:-true}"

NODE0_RPC_PORT="${NODE0_RPC_PORT:-26657}"
NODE0_P2P_PORT="${NODE0_P2P_PORT:-26656}"
NODE0_GRPC_PORT="${NODE0_GRPC_PORT:-9090}"
NODE0_API_PORT="${NODE0_API_PORT:-1317}"

NODE1_RPC_PORT="${NODE1_RPC_PORT:-26667}"
NODE1_P2P_PORT="${NODE1_P2P_PORT:-26666}"
NODE1_GRPC_PORT="${NODE1_GRPC_PORT:-9091}"
NODE1_API_PORT="${NODE1_API_PORT:-1318}"

NODE0_RPC="http://127.0.0.1:${NODE0_RPC_PORT}"

CW721_WASM="${CW721_WASM:-}"
CW721_SOURCE_DIR="${CW721_SOURCE_DIR:-$ROOT_DIR/.cache/cw-nfts}"
CW721_SOURCE_REF="${CW721_SOURCE_REF:-v0.21.0}"
CW721_BUILD_WASM_PATH="$CW721_SOURCE_DIR/target/wasm32-unknown-unknown/release/cw721_base.wasm"
CW721_RELEASE_URL="${CW721_RELEASE_URL:-https://github.com/public-awesome/cw-nfts/releases/download/${CW721_SOURCE_REF}/cw721_base.wasm}"
CW721_RELEASE_WASM_PATH="${CW721_RELEASE_WASM_PATH:-$ROOT_DIR/.cache/cw721_base.${CW721_SOURCE_REF}.wasm}"
WASM_UPLOAD_LIMIT_BYTES="${WASM_UPLOAD_LIMIT_BYTES:-819200}"
WASM_RUST_TOOLCHAIN="${WASM_RUST_TOOLCHAIN:-stable}"
WASM_RUSTFLAGS_COMPAT="${WASM_RUSTFLAGS_COMPAT:--C target-cpu=mvp -C target-feature=-bulk-memory,-reference-types,-multivalue,-sign-ext -C link-arg=-s}"
PREFER_OPTIMIZER_AUCTION_WASM="${PREFER_OPTIMIZER_AUCTION_WASM:-true}"
OPTIMIZER_IMAGE="${OPTIMIZER_IMAGE:-cosmwasm/optimizer:0.17.0}"
AUCTION_OPTIMIZED_WASM="${AUCTION_OPTIMIZED_WASM:-$ROOT_DIR/artifacts/mega_mad_scientist.wasm}"
AUCTION_WASM="${AUCTION_WASM:-$AUCTION_OPTIMIZED_WASM}"
FORCE_AUCTION_REBUILD="${FORCE_AUCTION_REBUILD:-false}"
DOCKER_BIN="${DOCKER_BIN:-}"
OPTIMIZER_LOG="${OPTIMIZER_LOG:-$ROOT_DIR/.cache/optimizer_build.log}"

NODE0_PID=""
NODE1_PID=""

log() {
  echo "[two-node-localnet] $*"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $1" >&2
    exit 1
  fi
}

resolve_docker_bin() {
  if [[ -n "$DOCKER_BIN" ]]; then
    return 0
  fi
  if command -v docker >/dev/null 2>&1; then
    DOCKER_BIN="$(command -v docker)"
    return 0
  fi
  if [[ -x "/Applications/Docker.app/Contents/Resources/bin/docker" ]]; then
    DOCKER_BIN="/Applications/Docker.app/Contents/Resources/bin/docker"
    return 0
  fi
  DOCKER_BIN=""
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

cleanup() {
  if [[ "$KEEP_RUNNING" == "true" ]]; then
    log "KEEP_RUNNING=true, leaving local nodes up."
    log "node0 rpc: $NODE0_RPC"
    log "node1 rpc: http://127.0.0.1:${NODE1_RPC_PORT}"
    return
  fi

  if [[ -n "$NODE0_PID" ]] && kill -0 "$NODE0_PID" >/dev/null 2>&1; then
    kill "$NODE0_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$NODE1_PID" ]] && kill -0 "$NODE1_PID" >/dev/null 2>&1; then
    kill "$NODE1_PID" >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT INT TERM

ensure_key() {
  local key="$1"
  local home="$2"
  if ! wasmd keys show "$key" --home "$home" --keyring-backend "$KEYRING_BACKEND" >/dev/null 2>&1; then
    wasmd keys add "$key" --home "$home" --keyring-backend "$KEYRING_BACKEND" >/dev/null 2>&1
  fi
}

addr_of() {
  local key="$1"
  local home="$2"
  wasmd keys show "$key" -a --home "$home" --keyring-backend "$KEYRING_BACKEND"
}

wait_rpc() {
  local url="$1"
  local max="${2:-90}"
  local i=0
  while (( i < max )); do
    i=$((i + 1))
    if curl -sf "$url/status" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_first_block() {
  local url="$1"
  local max="${2:-90}"
  local i=0
  local h
  while (( i < max )); do
    i=$((i + 1))
    h="$(
      curl -sf "$url/status" 2>/dev/null \
        | jq -r '.result.sync_info.latest_block_height // "0"' 2>/dev/null \
        || echo "0"
    )"
    if [[ "$h" =~ ^[0-9]+$ ]] && (( h > 0 )); then
      return 0
    fi
    sleep 1
  done
  return 1
}

assert_wasm_size_ok() {
  local wasm_path="$1"
  local sz

  sz="$(wc -c < "$wasm_path" | tr -d ' ')"
  if [[ "$sz" =~ ^[0-9]+$ ]] && (( sz > WASM_UPLOAD_LIMIT_BYTES )); then
    echo "ERROR: wasm artifact is ${sz} bytes, above local upload limit ${WASM_UPLOAD_LIMIT_BYTES}: $wasm_path" >&2
    exit 1
  fi
}

resolve_cw721_wasm() {
  if [[ -n "$CW721_WASM" ]]; then
    if [[ -f "$CW721_WASM" ]]; then
      assert_wasm_size_ok "$CW721_WASM"
      echo "$CW721_WASM"
      return 0
    fi
    echo "ERROR: CW721_WASM was set but file was not found: $CW721_WASM" >&2
    exit 1
  fi

  if [[ -f "$ROOT_DIR/artifacts/cw721_base.wasm" ]]; then
    assert_wasm_size_ok "$ROOT_DIR/artifacts/cw721_base.wasm"
    echo "$ROOT_DIR/artifacts/cw721_base.wasm"
    return 0
  fi

  if [[ -f "$CW721_RELEASE_WASM_PATH" ]]; then
    assert_wasm_size_ok "$CW721_RELEASE_WASM_PATH"
    echo "$CW721_RELEASE_WASM_PATH"
    return 0
  fi

  if [[ "$AUTO_BUILD_CW721" != "true" ]]; then
    echo "ERROR: cw721_base.wasm not found and AUTO_BUILD_CW721=false." >&2
    echo "Set CW721_WASM=/path/to/cw721_base.wasm and retry." >&2
    exit 1
  fi

  echo "[two-node-localnet] downloading prebuilt cw721_base.wasm from release" >&2
  mkdir -p "$ROOT_DIR/.cache"
  if curl -L --fail --retry 3 --retry-delay 1 "$CW721_RELEASE_URL" -o "$CW721_RELEASE_WASM_PATH" >/dev/null 2>&1; then
    assert_wasm_size_ok "$CW721_RELEASE_WASM_PATH"
    echo "$CW721_RELEASE_WASM_PATH"
    return 0
  fi

  echo "[two-node-localnet] release download failed; building from public-awesome/cw-nfts source" >&2
  mkdir -p "$ROOT_DIR/.cache"
  if [[ ! -d "$CW721_SOURCE_DIR/.git" ]]; then
    git clone --depth 1 --branch "$CW721_SOURCE_REF" https://github.com/public-awesome/cw-nfts.git "$CW721_SOURCE_DIR"
  else
    (
      cd "$CW721_SOURCE_DIR"
      git fetch --depth 1 origin "$CW721_SOURCE_REF" >/dev/null 2>&1 || true
      git checkout --force "$CW721_SOURCE_REF" >/dev/null 2>&1 || true
    )
  fi

  (
    cd "$CW721_SOURCE_DIR"
    rustup toolchain install "$WASM_RUST_TOOLCHAIN" >/dev/null
    ensure_edition2024_capable_toolchain
    rustup target add wasm32-unknown-unknown --toolchain "$WASM_RUST_TOOLCHAIN" >/dev/null
    # Build with size-oriented release settings to fit default local wasmd upload limit.
    CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
    CARGO_PROFILE_RELEASE_LTO=true \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_PANIC=abort \
    RUSTFLAGS="${WASM_RUSTFLAGS_COMPAT}" \
    cargo +"$WASM_RUST_TOOLCHAIN" build --release --target wasm32-unknown-unknown -p cw721-base
  )

  if [[ ! -f "$CW721_BUILD_WASM_PATH" ]]; then
    echo "ERROR: expected built wasm not found: $CW721_BUILD_WASM_PATH" >&2
    exit 1
  fi

  assert_wasm_size_ok "$CW721_BUILD_WASM_PATH"

  echo "$CW721_BUILD_WASM_PATH"
}

resolve_auction_wasm_inputs() {
  if [[ -f "$AUCTION_WASM" ]]; then
    log "using auction wasm: $AUCTION_WASM"
    return 0
  fi

  resolve_docker_bin
  if [[ "$PREFER_OPTIMIZER_AUCTION_WASM" == "true" ]] && [[ -n "$DOCKER_BIN" ]]; then
    log "auction wasm missing at $AUCTION_WASM; attempting optimizer build via $OPTIMIZER_IMAGE"
    mkdir -p "$ROOT_DIR/.cache"
    mkdir -p "$ROOT_DIR/artifacts"
    if "$DOCKER_BIN" run --rm -v "$ROOT_DIR":/code \
      --mount type=volume,source=cosmwasm_target_cache,target=/target \
      --mount type=volume,source=cosmwasm_registry_cache,target=/usr/local/cargo/registry \
      "$OPTIMIZER_IMAGE" >"$OPTIMIZER_LOG" 2>&1 && [[ -f "$AUCTION_WASM" ]]; then
      log "using optimizer-built auction wasm: $AUCTION_WASM"
      return 0
    fi
    log "optimizer build unavailable or failed (see $OPTIMIZER_LOG); falling back to rust build path"
  elif [[ "$PREFER_OPTIMIZER_AUCTION_WASM" == "true" ]]; then
    log "docker CLI not found; skipping optimizer build path"
  fi

  AUCTION_WASM="target/wasm32-unknown-unknown/release/mega_mad_scientist.wasm"
  FORCE_AUCTION_REBUILD="true"
  log "using rust build fallback for auction wasm: $AUCTION_WASM"
}

log "checking prerequisites"
require_cmd wasmd
require_cmd jq
require_cmd curl
require_cmd git
require_cmd rustup
require_cmd cargo

CW721_WASM_PATH="$(resolve_cw721_wasm | tail -n1)"
log "using cw721 wasm: $CW721_WASM_PATH"
resolve_auction_wasm_inputs

if [[ "$RESET_LOCALNET" == "true" ]]; then
  log "resetting localnet dir: $LOCALNET_DIR"
  rm -rf "$LOCALNET_DIR"
fi

mkdir -p "$LOG_DIR"

if [[ ! -f "$NODE0_HOME/config/genesis.json" ]]; then
  log "initializing node homes"
  wasmd init node0 --chain-id "$CHAIN_ID" --home "$NODE0_HOME" >/dev/null 2>&1
  wasmd init node1 --chain-id "$CHAIN_ID" --home "$NODE1_HOME" >/dev/null 2>&1

  ensure_key validator "$NODE0_HOME"
  ensure_key admin "$NODE0_HOME"
  ensure_key bidder1 "$NODE0_HOME"
  ensure_key bidder2 "$NODE0_HOME"

  VAL0_ADDR="$(addr_of validator "$NODE0_HOME")"
  ADMIN_ADDR="$(addr_of admin "$NODE0_HOME")"
  BIDDER1_ADDR="$(addr_of bidder1 "$NODE0_HOME")"
  BIDDER2_ADDR="$(addr_of bidder2 "$NODE0_HOME")"

  wasmd genesis add-genesis-account "$VAL0_ADDR" "100000000000${DENOM}" --home "$NODE0_HOME" >/dev/null 2>&1
  wasmd genesis add-genesis-account "$ADMIN_ADDR" "100000000000${DENOM}" --home "$NODE0_HOME" >/dev/null 2>&1
  wasmd genesis add-genesis-account "$BIDDER1_ADDR" "100000000000${DENOM}" --home "$NODE0_HOME" >/dev/null 2>&1
  wasmd genesis add-genesis-account "$BIDDER2_ADDR" "100000000000${DENOM}" --home "$NODE0_HOME" >/dev/null 2>&1

  wasmd genesis gentx validator "50000000000${DENOM}" \
    --chain-id "$CHAIN_ID" \
    --home "$NODE0_HOME" \
    --keyring-backend "$KEYRING_BACKEND" >/dev/null 2>&1

  wasmd genesis collect-gentxs --home "$NODE0_HOME" >/dev/null 2>&1
  wasmd genesis validate --home "$NODE0_HOME" >/dev/null 2>&1

  cp "$NODE0_HOME/config/genesis.json" "$NODE1_HOME/config/genesis.json"
fi

NODE0_ID="$(wasmd tendermint show-node-id --home "$NODE0_HOME")"
NODE1_ID="$(wasmd tendermint show-node-id --home "$NODE1_HOME")"

NODE0_PEERS="${NODE1_ID}@127.0.0.1:${NODE1_P2P_PORT}"
NODE1_PEERS="${NODE0_ID}@127.0.0.1:${NODE0_P2P_PORT}"

log "starting node0"
wasmd start \
  --home "$NODE0_HOME" \
  --minimum-gas-prices "0${DENOM}" \
  --rpc.laddr "tcp://127.0.0.1:${NODE0_RPC_PORT}" \
  --p2p.laddr "tcp://127.0.0.1:${NODE0_P2P_PORT}" \
  --grpc.address "127.0.0.1:${NODE0_GRPC_PORT}" \
  --api.address "tcp://127.0.0.1:${NODE0_API_PORT}" \
  --p2p.persistent_peers "$NODE0_PEERS" \
  >"$LOG_DIR/node0.log" 2>&1 &
NODE0_PID=$!

log "starting node1"
wasmd start \
  --home "$NODE1_HOME" \
  --minimum-gas-prices "0${DENOM}" \
  --rpc.laddr "tcp://127.0.0.1:${NODE1_RPC_PORT}" \
  --p2p.laddr "tcp://127.0.0.1:${NODE1_P2P_PORT}" \
  --grpc.address "127.0.0.1:${NODE1_GRPC_PORT}" \
  --api.address "tcp://127.0.0.1:${NODE1_API_PORT}" \
  --p2p.persistent_peers "$NODE1_PEERS" \
  >"$LOG_DIR/node1.log" 2>&1 &
NODE1_PID=$!

log "waiting for node0 rpc: $NODE0_RPC"
if ! wait_rpc "$NODE0_RPC" 120; then
  echo "ERROR: node0 rpc did not come up in time. See $LOG_DIR/node0.log" >&2
  exit 1
fi
log "waiting for node0 first block"
if ! wait_first_block "$NODE0_RPC" 120; then
  echo "ERROR: node0 did not produce first block in time. See $LOG_DIR/node0.log" >&2
  exit 1
fi

log "waiting for node1 rpc: http://127.0.0.1:${NODE1_RPC_PORT}"
if ! wait_rpc "http://127.0.0.1:${NODE1_RPC_PORT}" 120; then
  echo "ERROR: node1 rpc did not come up in time. See $LOG_DIR/node1.log" >&2
  exit 1
fi
log "waiting for node1 first block"
if ! wait_first_block "http://127.0.0.1:${NODE1_RPC_PORT}" 120; then
  echo "ERROR: node1 did not produce first block in time. See $LOG_DIR/node1.log" >&2
  exit 1
fi

log "running e2e against node0"
CHAIN_ID="$CHAIN_ID" \
NODE="$NODE0_RPC" \
WASMD_HOME="$NODE0_HOME" \
KEYRING_BACKEND="$KEYRING_BACKEND" \
DENOM="$DENOM" \
ADMIN_KEY="admin" \
BIDDER1_KEY="bidder1" \
BIDDER2_KEY="bidder2" \
FUNDER_KEY="validator" \
CW721_WASM="$CW721_WASM_PATH" \
AUCTION_WASM="$AUCTION_WASM" \
FORCE_AUCTION_REBUILD="$FORCE_AUCTION_REBUILD" \
WASM_RUST_TOOLCHAIN="$WASM_RUST_TOOLCHAIN" \
WASM_RUSTFLAGS_COMPAT="$WASM_RUSTFLAGS_COMPAT" \
"$ROOT_DIR/localnet_e2e.sh"

log "two-node localnet e2e completed successfully (local-only test chain)"
