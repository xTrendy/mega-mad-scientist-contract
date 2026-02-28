#!/bin/bash
# ═══════════════════════════════════════════════════════════════════════
# Cosmic Mad Scientist — Cosmos Hub Deployment Script
# ═══════════════════════════════════════════════════════════════════════
#
# Usage:
#   ./deploy.sh                  (mainnet — requires explicit confirmation)
#   ./deploy.sh --testnet        (theta testnet)
#   ./deploy.sh --dry-run        (print commands without executing)
#
# Prerequisites:
#   - gaiad installed and on PATH
#   - jq installed and on PATH
#   - Wallet key imported: gaiad keys add <WALLET_NAME> --recover
#   - Sufficient ATOM for gas (~0.5 ATOM for store + instantiate)
#   - Contract compiled: docker run --rm -v "$(pwd)":/code \
#       cosmwasm/optimizer:0.15.0
#
# ═══════════════════════════════════════════════════════════════════════

set -euo pipefail

# ── Colors ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# ── Configuration (edit these!) ───────────────────────────────────────

# Wallet name as it appears in `gaiad keys list`
WALLET_NAME="${WALLET_NAME:-deployer}"

# Collection addresses (MUST be set before deploying)
MAD_COLLECTION="${MAD_COLLECTION:-}"
MEGA_COLLECTION="${MEGA_COLLECTION:-}"

# Contract admin (defaults to deployer wallet address)
ADMIN_ADDRESS="${ADMIN_ADDRESS:-}"

# Instantiate config (defaults match contract defaults)
DEFAULT_MIN_BID="${DEFAULT_MIN_BID:-1}"
ANTI_SNIPE_WINDOW="${ANTI_SNIPE_WINDOW:-300}"
ANTI_SNIPE_EXTENSION="${ANTI_SNIPE_EXTENSION:-300}"
MAX_EXTENSION="${MAX_EXTENSION:-86400}"
MAX_BIDDERS_PER_AUCTION="${MAX_BIDDERS_PER_AUCTION:-100}"
MAX_STAGING_SIZE="${MAX_STAGING_SIZE:-50}"
MAX_NFTS_PER_BID="${MAX_NFTS_PER_BID:-50}"

# Gas settings
GAS_PRICES="0.025uatom"
GAS_ADJUSTMENT="1.3"

# Path to optimized wasm binary
WASM_PATH="./artifacts/mega_mad_scientist.wasm"

# TX confirmation polling settings
TX_POLL_INTERVAL=5      # seconds between polls
TX_POLL_MAX_ATTEMPTS=24 # 24 * 5s = 2 minute max wait

# ── Network Configuration ─────────────────────────────────────────────

CHAIN_ID="cosmoshub-4"
NODE="https://rpc.cosmos.directory/cosmoshub"
LABEL="mega-mad-scientist-v0.1.0"

DRY_RUN=false
IS_TESTNET=false

for arg in "$@"; do
    case $arg in
        --testnet)
            IS_TESTNET=true
            CHAIN_ID="theta-testnet-001"
            NODE="https://rpc.sentry-01.theta-testnet.polypore.xyz:443"
            LABEL="mega-mad-scientist-v0.1.0-testnet"
            echo -e "${YELLOW}Using Cosmos Hub TESTNET (theta)${NC}"
            ;;
        --dry-run)
            DRY_RUN=true
            echo -e "${YELLOW}DRY RUN — commands will be printed but not executed${NC}"
            ;;
        *)
            echo -e "${RED}Unknown flag: $arg${NC}"
            echo "Usage: ./deploy.sh [--testnet] [--dry-run]"
            exit 1
            ;;
    esac
done

# ── Helper Functions ──────────────────────────────────────────────────

log_cmd() {
    # Print the command for visibility, but do NOT use eval
    echo -e "${CYAN}$ $*${NC}"
}

confirm_or_abort() {
    local prompt="$1"
    echo ""
    echo -e "${YELLOW}$prompt${NC}"
    read -r -p "Type 'yes' to continue: " response
    if [ "$response" != "yes" ]; then
        echo -e "${RED}Aborted.${NC}"
        exit 0
    fi
}

check_required() {
    local var_name=$1
    local var_value=$2
    local description=$3
    if [ -z "$var_value" ]; then
        echo -e "${RED}ERROR: $description ($var_name) is not set.${NC}"
        echo "  Set it via environment variable: export $var_name=cosmos1..."
        exit 1
    fi
}

# Poll for TX confirmation instead of blind sleep
wait_for_tx() {
    local tx_hash=$1
    local attempt=0

    echo -e "${YELLOW}  Waiting for TX $tx_hash to confirm...${NC}"
    while [ $attempt -lt $TX_POLL_MAX_ATTEMPTS ]; do
        attempt=$((attempt + 1))
        sleep "$TX_POLL_INTERVAL"

        local result
        result=$(gaiad query tx "$tx_hash" --node "$NODE" --output json 2>/dev/null || echo "")
        if [ -n "$result" ]; then
            # Check if TX succeeded (code 0 = success)
            local code
            code=$(echo "$result" | jq -r '.code // 0')
            if [ "$code" != "0" ]; then
                local raw_log
                raw_log=$(echo "$result" | jq -r '.raw_log // "unknown error"')
                echo -e "${RED}  TX FAILED (code $code): $raw_log${NC}"
                exit 1
            fi
            echo -e "${GREEN}  TX confirmed in ~$((attempt * TX_POLL_INTERVAL))s${NC}"
            echo "$result"
            return 0
        fi
        echo -e "  ... polling ($attempt/$TX_POLL_MAX_ATTEMPTS)"
    done

    echo -e "${RED}ERROR: TX $tx_hash not confirmed after $((TX_POLL_MAX_ATTEMPTS * TX_POLL_INTERVAL))s${NC}"
    echo "  Check manually: gaiad query tx $tx_hash --node $NODE"
    exit 1
}

# ── Preflight Checks ─────────────────────────────────────────────────

echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Cosmic Mad Scientist — Deployment Script${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo ""

# Check gaiad is installed
if ! command -v gaiad &> /dev/null; then
    echo -e "${RED}ERROR: gaiad not found. Install it first:${NC}"
    echo "  https://hub.cosmos.network/getting-started/installation"
    exit 1
fi

# Check jq is installed
if ! command -v jq &> /dev/null; then
    echo -e "${RED}ERROR: jq not found. Install it first:${NC}"
    echo "  brew install jq  (macOS)"
    echo "  apt install jq   (Linux)"
    exit 1
fi

# Check wasm binary exists
if [ ! -f "$WASM_PATH" ]; then
    echo -e "${RED}ERROR: Compiled wasm not found at $WASM_PATH${NC}"
    echo "  Build it with:"
    echo "    docker run --rm -v \"\$(pwd)\":/code cosmwasm/optimizer:0.15.0"
    exit 1
fi

# Show wasm checksum for verification
WASM_CHECKSUM=$(sha256sum "$WASM_PATH" | awk '{print $1}')
echo -e "  Wasm checksum:  ${CYAN}${WASM_CHECKSUM}${NC}"
WASM_SIZE=$(du -h "$WASM_PATH" | awk '{print $1}')
echo -e "  Wasm size:      ${CYAN}${WASM_SIZE}${NC}"

# Check required addresses
check_required "MAD_COLLECTION" "$MAD_COLLECTION" "Standard Mad Scientist CW721 collection address"
check_required "MEGA_COLLECTION" "$MEGA_COLLECTION" "Cosmic Mad Scientist CW721 collection address"

# Validate addresses look like bech32
for addr_var in MAD_COLLECTION MEGA_COLLECTION; do
    addr_val="${!addr_var}"
    if [[ ! "$addr_val" =~ ^cosmos1[a-z0-9]{38,58}$ ]]; then
        echo -e "${YELLOW}WARNING: $addr_var ($addr_val) does not look like a valid cosmos1 address.${NC}"
        echo "  Expected format: cosmos1<38-58 lowercase alphanumeric chars>"
        if [ "$DRY_RUN" = false ]; then
            confirm_or_abort "Continue anyway?"
        fi
    fi
done

# Resolve admin address
if [ -z "$ADMIN_ADDRESS" ]; then
    if [ "$DRY_RUN" = true ]; then
        ADMIN_ADDRESS="<WALLET_ADDRESS>"
    else
        ADMIN_ADDRESS=$(gaiad keys show "$WALLET_NAME" -a 2>/dev/null || true)
        if [ -z "$ADMIN_ADDRESS" ]; then
            echo -e "${RED}ERROR: Could not resolve wallet '$WALLET_NAME'. Import it first:${NC}"
            echo "  gaiad keys add $WALLET_NAME --recover"
            exit 1
        fi
    fi
fi

echo ""
echo -e "  Chain ID:       ${CYAN}$CHAIN_ID${NC}"
echo -e "  Node:           ${CYAN}$NODE${NC}"
echo -e "  Wallet:         ${CYAN}$WALLET_NAME${NC}"
echo -e "  Admin:          ${CYAN}$ADMIN_ADDRESS${NC}"
echo -e "  Mad Collection: ${CYAN}$MAD_COLLECTION${NC}"
echo -e "  Cosmic Collection:${CYAN}$MEGA_COLLECTION${NC}"
echo -e "  Wasm:           ${CYAN}$WASM_PATH${NC}"
echo ""

# Mainnet requires explicit confirmation
if [ "$DRY_RUN" = false ]; then
    if [ "$IS_TESTNET" = false ]; then
        echo -e "${RED}╔═══════════════════════════════════════════════════╗${NC}"
        echo -e "${RED}║         DEPLOYING TO MAINNET (cosmoshub-4)        ║${NC}"
        echo -e "${RED}║    This will spend real ATOM. Double-check above. ║${NC}"
        echo -e "${RED}╚═══════════════════════════════════════════════════╝${NC}"
        confirm_or_abort "Deploy to MAINNET?"
    else
        echo -e "${YELLOW}Deploying to testnet in 5 seconds... (Ctrl+C to cancel)${NC}"
        sleep 5
    fi
fi

# ═════════════════════════════════════════════════════════════════════
# STEP 1: Store the contract on-chain
# ═════════════════════════════════════════════════════════════════════

echo ""
echo -e "${GREEN}[1/3] Storing contract on-chain...${NC}"

if [ "$DRY_RUN" = true ]; then
    log_cmd gaiad tx wasm store "$WASM_PATH" \
        --from "$WALLET_NAME" \
        --chain-id "$CHAIN_ID" \
        --node "$NODE" \
        --gas-prices "$GAS_PRICES" \
        --gas auto \
        --gas-adjustment "$GAS_ADJUSTMENT" \
        --output json -y
    echo -e "${YELLOW}  (skipped — dry run)${NC}"
    CODE_ID="<CODE_ID>"
else
    STORE_RESULT=$(gaiad tx wasm store "$WASM_PATH" \
        --from "$WALLET_NAME" \
        --chain-id "$CHAIN_ID" \
        --node "$NODE" \
        --gas-prices "$GAS_PRICES" \
        --gas auto \
        --gas-adjustment "$GAS_ADJUSTMENT" \
        --output json \
        -y)

    STORE_TX=$(echo "$STORE_RESULT" | jq -r '.txhash')
    if [ -z "$STORE_TX" ] || [ "$STORE_TX" = "null" ]; then
        echo -e "${RED}ERROR: No txhash returned from store command.${NC}"
        echo "$STORE_RESULT" | jq .
        exit 1
    fi
    echo -e "  Store TX: ${CYAN}$STORE_TX${NC}"

    TX_RESULT=$(wait_for_tx "$STORE_TX")

    # Extract code_id — try both old (.logs) and new (.events) formats
    CODE_ID=$(echo "$TX_RESULT" | jq -r '
        (.logs[0].events[]? | select(.type=="store_code") | .attributes[] | select(.key=="code_id") | .value) //
        ([.events[]? | select(.type=="store_code") | .attributes[] | select(.key=="code_id") | .value] | .[0]) //
        empty
    ' 2>/dev/null || echo "")

    if [ -z "$CODE_ID" ] || [ "$CODE_ID" = "null" ]; then
        echo -e "${RED}ERROR: Could not extract code_id from TX. Check TX manually:${NC}"
        echo "  gaiad query tx $STORE_TX --node $NODE"
        exit 1
    fi

    echo -e "  ${GREEN}Code ID: $CODE_ID${NC}"
fi

# ═════════════════════════════════════════════════════════════════════
# STEP 2: Instantiate the contract
# ═════════════════════════════════════════════════════════════════════

echo ""
echo -e "${GREEN}[2/3] Instantiating contract...${NC}"

# Build the init message — use jq to ensure valid JSON
INIT_MSG=$(jq -n \
    --arg admin "$ADMIN_ADDRESS" \
    --arg mad "$MAD_COLLECTION" \
    --arg mega "$MEGA_COLLECTION" \
    --argjson min_bid "$DEFAULT_MIN_BID" \
    --argjson snipe_win "$ANTI_SNIPE_WINDOW" \
    --argjson snipe_ext "$ANTI_SNIPE_EXTENSION" \
    --argjson max_ext "$MAX_EXTENSION" \
    --argjson max_bidders "$MAX_BIDDERS_PER_AUCTION" \
    --argjson max_staging "$MAX_STAGING_SIZE" \
    --argjson max_nfts "$MAX_NFTS_PER_BID" \
    '{
        admin: $admin,
        mad_scientist_collection: $mad,
        mega_mad_scientist_collection: $mega,
        default_min_bid: $min_bid,
        anti_snipe_window: $snipe_win,
        anti_snipe_extension: $snipe_ext,
        max_extension: $max_ext,
        max_bidders_per_auction: $max_bidders,
        max_staging_size: $max_staging,
        max_nfts_per_bid: $max_nfts
    }')

echo -e "  Init msg:"
echo "$INIT_MSG" | jq .

if [ "$DRY_RUN" = true ]; then
    log_cmd gaiad tx wasm instantiate "$CODE_ID" "'$INIT_MSG'" \
        --from "$WALLET_NAME" \
        --label "$LABEL" \
        --admin "$ADMIN_ADDRESS" \
        --chain-id "$CHAIN_ID" \
        --node "$NODE" \
        --gas-prices "$GAS_PRICES" \
        --gas auto \
        --gas-adjustment "$GAS_ADJUSTMENT" \
        --output json -y
    echo -e "${YELLOW}  (skipped — dry run)${NC}"
    CONTRACT_ADDRESS="<CONTRACT_ADDRESS>"
else
    INIT_RESULT=$(gaiad tx wasm instantiate "$CODE_ID" "$INIT_MSG" \
        --from "$WALLET_NAME" \
        --label "$LABEL" \
        --admin "$ADMIN_ADDRESS" \
        --chain-id "$CHAIN_ID" \
        --node "$NODE" \
        --gas-prices "$GAS_PRICES" \
        --gas auto \
        --gas-adjustment "$GAS_ADJUSTMENT" \
        --output json \
        -y)

    INIT_TX=$(echo "$INIT_RESULT" | jq -r '.txhash')
    if [ -z "$INIT_TX" ] || [ "$INIT_TX" = "null" ]; then
        echo -e "${RED}ERROR: No txhash returned from instantiate command.${NC}"
        echo "$INIT_RESULT" | jq .
        exit 1
    fi
    echo -e "  Instantiate TX: ${CYAN}$INIT_TX${NC}"

    TX_RESULT=$(wait_for_tx "$INIT_TX")

    # Extract contract address — try both old and new event formats
    CONTRACT_ADDRESS=$(echo "$TX_RESULT" | jq -r '
        (.logs[0].events[]? | select(.type=="instantiate") | .attributes[] | select(.key=="_contract_address") | .value) //
        ([.events[]? | select(.type=="instantiate") | .attributes[] | select(.key=="_contract_address") | .value] | .[0]) //
        empty
    ' 2>/dev/null || echo "")

    if [ -z "$CONTRACT_ADDRESS" ] || [ "$CONTRACT_ADDRESS" = "null" ]; then
        echo -e "${RED}ERROR: Could not extract contract address. Check TX manually:${NC}"
        echo "  gaiad query tx $INIT_TX --node $NODE"
        exit 1
    fi

    echo -e "  ${GREEN}Contract Address: $CONTRACT_ADDRESS${NC}"
fi

# ═════════════════════════════════════════════════════════════════════
# STEP 3: Verify the deployment
# ═════════════════════════════════════════════════════════════════════

echo ""
echo -e "${GREEN}[3/3] Verifying deployment...${NC}"

QUERY_MSG='{"get_config":{}}'

if [ "$DRY_RUN" = true ]; then
    log_cmd gaiad query wasm contract-state smart "$CONTRACT_ADDRESS" "'$QUERY_MSG'" \
        --node "$NODE" --output json
    echo -e "${YELLOW}  (skipped — dry run)${NC}"
else
    CONFIG_RESULT=$(gaiad query wasm contract-state smart "$CONTRACT_ADDRESS" "$QUERY_MSG" \
        --node "$NODE" \
        --output json)

    echo ""
    echo -e "${GREEN}Contract config:${NC}"
    echo "$CONFIG_RESULT" | jq '.data'

    # Validate key fields
    RETURNED_ADMIN=$(echo "$CONFIG_RESULT" | jq -r '.data.admin')
    RETURNED_MAD=$(echo "$CONFIG_RESULT" | jq -r '.data.mad_scientist_collection')
    RETURNED_MEGA=$(echo "$CONFIG_RESULT" | jq -r '.data.mega_mad_scientist_collection')
    RETURNED_PAUSED=$(echo "$CONFIG_RESULT" | jq -r '.data.paused')
    RETURNED_MIN_BID=$(echo "$CONFIG_RESULT" | jq -r '.data.default_min_bid')

    echo ""
    VERIFY_OK=true
    for check in \
        "admin:$RETURNED_ADMIN:$ADMIN_ADDRESS" \
        "mad_collection:$RETURNED_MAD:$MAD_COLLECTION" \
        "mega_collection:$RETURNED_MEGA:$MEGA_COLLECTION" \
        "paused:$RETURNED_PAUSED:false" \
        "default_min_bid:$RETURNED_MIN_BID:$DEFAULT_MIN_BID"; do
        IFS=':' read -r field got expected <<< "$check"
        if [ "$got" = "$expected" ]; then
            echo -e "  ${GREEN}✓${NC} $field = $got"
        else
            echo -e "  ${RED}✗${NC} $field: expected $expected, got $got"
            VERIFY_OK=false
        fi
    done

    if [ "$VERIFY_OK" = true ]; then
        echo -e "\n${GREEN}  All fields verified.${NC}"
    else
        echo -e "\n${RED}  Some fields don't match — review above.${NC}"
    fi
fi

# ═════════════════════════════════════════════════════════════════════
# Summary
# ═════════════════════════════════════════════════════════════════════

echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Deployment Complete!${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo ""
echo -e "  Chain:     ${CYAN}$CHAIN_ID${NC}"
echo -e "  Code ID:   ${CYAN}$CODE_ID${NC}"
echo -e "  Contract:  ${CYAN}$CONTRACT_ADDRESS${NC}"
echo -e "  Admin:     ${CYAN}$ADMIN_ADDRESS${NC}"
echo -e "  Checksum:  ${CYAN}$WASM_CHECKSUM${NC}"
echo ""
echo -e "  ${YELLOW}Save these values! You'll need the contract address${NC}"
echo -e "  ${YELLOW}to interact with the contract.${NC}"
echo ""

# Write deployment info to artifacts/ alongside the wasm
DEPLOY_DIR="./artifacts"
mkdir -p "$DEPLOY_DIR"
DEPLOY_FILE="${DEPLOY_DIR}/deployment-${CHAIN_ID}-$(date +%Y%m%d-%H%M%S).json"
cat > "$DEPLOY_FILE" <<DEPLOY_EOF
{
  "chain_id": "$CHAIN_ID",
  "code_id": "$CODE_ID",
  "contract_address": "$CONTRACT_ADDRESS",
  "admin": "$ADMIN_ADDRESS",
  "mad_collection": "$MAD_COLLECTION",
  "mega_collection": "$MEGA_COLLECTION",
  "label": "$LABEL",
  "wasm_checksum": "$WASM_CHECKSUM",
  "deployed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "config": {
    "default_min_bid": $DEFAULT_MIN_BID,
    "anti_snipe_window": $ANTI_SNIPE_WINDOW,
    "anti_snipe_extension": $ANTI_SNIPE_EXTENSION,
    "max_extension": $MAX_EXTENSION,
    "max_bidders_per_auction": $MAX_BIDDERS_PER_AUCTION,
    "max_staging_size": $MAX_STAGING_SIZE,
    "max_nfts_per_bid": $MAX_NFTS_PER_BID
  }
}
DEPLOY_EOF

echo -e "  Deployment info saved to: ${CYAN}$DEPLOY_FILE${NC}"
echo ""
