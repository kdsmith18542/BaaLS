#!/usr/bin/env bash
# anchor-deployment.sh
# Phase M.1 helper: build a BaaLS deployment manifest and optionally upload to Irys/Arweave.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  scripts/anchor-deployment.sh [options] [-- <extra irys args>]

Options:
  --api-url <url>          BaaLS API base URL (default: http://127.0.0.1:18080)
  --config <path>          Path to config.toml (default: /etc/baals/config.toml)
  --data-dir <path>        Node data dir (default: /var/lib/baals)
  --output-dir <path>      Manifest output directory (default: ./deploy)
  --anchors-file <path>    Tracked anchors file (default: ./deploy/arweave-anchors.json)
  --baalsd-bin <path>      baalsd binary for node-id fallback (default: baalsd)
  --irys-bin <path>        irys binary (default: irys)
  --irys-network <name>    Irys network (default: devnet)
  --irys-token <name>      Irys payment token (default: ethereum)
  --irys-provider-url <u>  Irys provider RPC URL (default: [oracle].evm_rpc)
  --irys-wallet <key>      Wallet/private key for irys -w
  --irys-wallet-env <name> Env var name containing irys wallet/private key
  --genesis-hash <hex>     Override genesis hash (skip API fetch for genesis)
  --node-id <id>           Override node ID
  --no-upload              Build manifest only, skip irys upload
  -h, --help               Show help

Examples:
  scripts/anchor-deployment.sh --no-upload
  scripts/anchor-deployment.sh --api-url https://baals.network -- --network devnet
EOF
}

fail() {
    echo "Error: $*" >&2
    exit 1
}

json_get_string() {
    local json="$1"
    local key="$2"
    if command -v jq >/dev/null 2>&1; then
        printf '%s' "$json" | jq -r --arg k "$key" '.[$k] // empty'
    else
        printf '%s' "$json" |
            sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" |
            head -n1
    fi
}

json_get_number() {
    local json="$1"
    local key="$2"
    if command -v jq >/dev/null 2>&1; then
        printf '%s' "$json" | jq -r --arg k "$key" '.[$k] // empty'
    else
        printf '%s' "$json" |
            sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\\([0-9][0-9]*\\).*/\\1/p" |
            head -n1
    fi
}

toml_get_string() {
    local file="$1"
    local key="$2"
    grep -E "^[[:space:]]*$key[[:space:]]*=" "$file" | tail -n1 |
        sed -E 's/^[^=]*=[[:space:]]*"([^"]*)".*/\1/' || true
}

toml_get_number() {
    local file="$1"
    local key="$2"
    grep -E "^[[:space:]]*$key[[:space:]]*=" "$file" | tail -n1 |
        sed -E 's/^[^=]*=[[:space:]]*([0-9]+).*/\1/' || true
}

sha256_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print $1}'
    else
        fail "sha256sum/shasum not found"
    fi
}

API_URL="${API_URL:-http://127.0.0.1:18080}"
CONFIG_PATH="${CONFIG_PATH:-/etc/baals/config.toml}"
DATA_DIR="${DATA_DIR:-/var/lib/baals}"
OUTPUT_DIR="${OUTPUT_DIR:-./deploy}"
ANCHORS_FILE="${ANCHORS_FILE:-./deploy/arweave-anchors.json}"
BAALSD_BIN="${BAALSD_BIN:-baalsd}"
IRYS_BIN="${IRYS_BIN:-irys}"
IRYS_NETWORK="${IRYS_NETWORK:-devnet}"
IRYS_TOKEN="${IRYS_TOKEN:-ethereum}"
IRYS_PROVIDER_URL="${IRYS_PROVIDER_URL:-}"
IRYS_WALLET="${IRYS_WALLET:-}"
IRYS_WALLET_ENV="${IRYS_WALLET_ENV:-}"
GENESIS_HASH_OVERRIDE=""
NODE_ID_OVERRIDE=""
NO_UPLOAD=0
IRYS_EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --api-url)
            API_URL="$2"
            shift 2
            ;;
        --config)
            CONFIG_PATH="$2"
            shift 2
            ;;
        --data-dir)
            DATA_DIR="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --anchors-file)
            ANCHORS_FILE="$2"
            shift 2
            ;;
        --baalsd-bin)
            BAALSD_BIN="$2"
            shift 2
            ;;
        --irys-bin)
            IRYS_BIN="$2"
            shift 2
            ;;
        --irys-network)
            IRYS_NETWORK="$2"
            shift 2
            ;;
        --irys-token)
            IRYS_TOKEN="$2"
            shift 2
            ;;
        --irys-provider-url)
            IRYS_PROVIDER_URL="$2"
            shift 2
            ;;
        --irys-wallet)
            IRYS_WALLET="$2"
            shift 2
            ;;
        --irys-wallet-env)
            IRYS_WALLET_ENV="$2"
            shift 2
            ;;
        --genesis-hash)
            GENESIS_HASH_OVERRIDE="$2"
            shift 2
            ;;
        --node-id)
            NODE_ID_OVERRIDE="$2"
            shift 2
            ;;
        --no-upload)
            NO_UPLOAD=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            IRYS_EXTRA_ARGS=("$@")
            break
            ;;
        *)
            fail "unknown option: $1"
            ;;
    esac
done

command -v curl >/dev/null 2>&1 || fail "curl is required"
[[ -f "$CONFIG_PATH" ]] || fail "config not found: $CONFIG_PATH"

mkdir -p "$OUTPUT_DIR"
mkdir -p "$(dirname "$ANCHORS_FILE")"

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
timestamp_slug="$(date -u +%Y%m%dT%H%M%SZ)"
manifest_path="$OUTPUT_DIR/baals-deployment-manifest-${timestamp_slug}.json"

if [[ -n "$GENESIS_HASH_OVERRIDE" ]]; then
    genesis_hash="$GENESIS_HASH_OVERRIDE"
else
    genesis_json="$(curl -fsS "${API_URL%/}/api/v1/blocks/0")" ||
        fail "unable to fetch genesis block from ${API_URL%/}/api/v1/blocks/0"
    genesis_hash="$(json_get_string "$genesis_json" "hash")"
    [[ -n "$genesis_hash" ]] || fail "could not parse genesis hash from API response"
fi

latest_json="$(curl -fsS "${API_URL%/}/api/v1/blocks/latest" 2>/dev/null || true)"
latest_height="$(json_get_number "$latest_json" "height")"
latest_hash="$(json_get_string "$latest_json" "hash")"

node_id=""
node_id_source=""
if [[ -n "$NODE_ID_OVERRIDE" ]]; then
    node_id="$NODE_ID_OVERRIDE"
    node_id_source="cli-override"
elif [[ -f "$DATA_DIR/node_id" ]]; then
    node_id="$(tr -d '\r\n' < "$DATA_DIR/node_id")"
    node_id_source="$DATA_DIR/node_id"
elif command -v "$BAALSD_BIN" >/dev/null 2>&1; then
    node_json="$("$BAALSD_BIN" --json admin export-node-id --data-dir "$DATA_DIR" 2>/dev/null || true)"
    parsed_node_id="$(json_get_string "$node_json" "node_id")"
    if [[ -n "$parsed_node_id" ]]; then
        node_id="$parsed_node_id"
        node_id_source="baalsd-admin-export-node-id"
    fi
fi

if [[ -z "$node_id" ]]; then
    node_id="$(hostname)"
    node_id_source="hostname-fallback"
    echo "Warning: node_id fell back to hostname ($node_id); pass --node-id for deterministic identity." >&2
fi

reward_distributor="$(toml_get_string "$CONFIG_PATH" "reward_distributor")"
evm_chain_id="$(toml_get_number "$CONFIG_PATH" "evm_chain_id")"
evm_private_key_env="$(toml_get_string "$CONFIG_PATH" "evm_private_key_env")"
evm_rpc="$(toml_get_string "$CONFIG_PATH" "evm_rpc")"
config_hash="$(sha256_file "$CONFIG_PATH")"

[[ -n "$reward_distributor" ]] || fail "reward_distributor missing in $CONFIG_PATH"
[[ -n "$evm_chain_id" ]] || fail "evm_chain_id missing in $CONFIG_PATH"
[[ -n "$evm_private_key_env" ]] || fail "evm_private_key_env missing in $CONFIG_PATH"

if [[ -z "$IRYS_PROVIDER_URL" && -n "$evm_rpc" ]]; then
    IRYS_PROVIDER_URL="$evm_rpc"
fi
if [[ -z "$IRYS_WALLET_ENV" && -n "$evm_private_key_env" ]]; then
    IRYS_WALLET_ENV="$evm_private_key_env"
fi
if [[ -z "$IRYS_WALLET" && -n "$IRYS_WALLET_ENV" ]]; then
    IRYS_WALLET="${!IRYS_WALLET_ENV-}"
    if [[ -z "$IRYS_WALLET" ]] && command -v systemctl >/dev/null 2>&1; then
        svc_env="$(systemctl show baalsd --property=Environment --value 2>/dev/null || true)"
        if [[ -z "$svc_env" ]]; then
            svc_env="$(sudo -n systemctl show baalsd --property=Environment --value 2>/dev/null || true)"
        fi
        if [[ -n "$svc_env" ]]; then
            IRYS_WALLET="$(
                printf '%s\n' "$svc_env" | tr ' ' '\n' |
                    awk -F= -v key="$IRYS_WALLET_ENV" '$1==key { print $2; exit }'
            )"
        fi
    fi
fi

cat >"$manifest_path" <<EOF
{
  "schema_version": "baals:deployment-anchor:v1",
  "generated_at_utc": "$timestamp",
  "api_url": "${API_URL%/}",
  "config_path": "$CONFIG_PATH",
  "data_dir": "$DATA_DIR",
  "chain": {
    "genesis_hash": "$genesis_hash",
    "latest_height": ${latest_height:-0},
    "latest_hash": "${latest_hash:-}",
    "node_id": "$node_id",
    "node_id_source": "$node_id_source"
  },
  "oracle": {
    "reward_distributor": "$reward_distributor",
    "evm_chain_id": $evm_chain_id,
    "evm_private_key_env": "$evm_private_key_env"
  },
  "config_hash_sha256": "$config_hash"
}
EOF

manifest_hash="$(sha256_file "$manifest_path")"

if [[ ! -f "$ANCHORS_FILE" ]]; then
    cat >"$ANCHORS_FILE" <<'EOF'
{
  "schema_version": "baals:arweave-anchors:v1",
  "anchors": []
}
EOF
fi

if [[ "$NO_UPLOAD" -eq 1 ]]; then
    echo "Manifest generated (upload skipped): $manifest_path"
    echo "Manifest SHA-256: $manifest_hash"
    exit 0
fi

command -v "$IRYS_BIN" >/dev/null 2>&1 || fail "irys binary not found: $IRYS_BIN"
if [[ "$IRYS_NETWORK" == "devnet" && -z "$IRYS_PROVIDER_URL" ]]; then
    fail "IRYS devnet selected but provider URL is empty (set --irys-provider-url or [oracle].evm_rpc)"
fi
if [[ -z "$IRYS_WALLET" ]]; then
    echo "Warning: no irys wallet provided; falling back to irys default wallet context." >&2
fi

upload_cmd=("$IRYS_BIN" "upload" "$manifest_path"
    "-n" "$IRYS_NETWORK"
    "-t" "$IRYS_TOKEN"
    "--tags" "App-Name" "BaaLS" "Type" "deployment-manifest" "Content-Type" "application/json")
if [[ -n "$IRYS_PROVIDER_URL" ]]; then
    upload_cmd+=("--provider-url" "$IRYS_PROVIDER_URL")
fi
if [[ -n "$IRYS_WALLET" ]]; then
    upload_cmd+=("-w" "$IRYS_WALLET")
fi
if [[ ${#IRYS_EXTRA_ARGS[@]} -gt 0 ]]; then
    upload_cmd+=("${IRYS_EXTRA_ARGS[@]}")
fi

upload_output="$("${upload_cmd[@]}" 2>&1)" || {
    echo "$upload_output" >&2
    fail "irys upload failed"
}

echo "$upload_output"

tx_id="$(printf '%s\n' "$upload_output" |
    sed -n 's#.*gateway.irys.xyz/\([A-Za-z0-9_-]\+\).*#\1#p' |
    tail -n1)"

if [[ -z "$tx_id" ]]; then
    tx_id="UNKNOWN"
fi

if command -v jq >/dev/null 2>&1; then
    tmp_file="$(mktemp)"
    jq \
        --arg ts "$timestamp" \
        --arg tx "$tx_id" \
        --arg manifest "$manifest_path" \
        --arg hash "$manifest_hash" \
        --arg genesis "$genesis_hash" \
        --arg node_id "$node_id" \
        --arg distributor "$reward_distributor" \
        '.anchors += [{
            "timestamp_utc": $ts,
            "tx_id": $tx,
            "manifest_path": $manifest,
            "manifest_sha256": $hash,
            "genesis_hash": $genesis,
            "node_id": $node_id,
            "reward_distributor": $distributor
        }]' "$ANCHORS_FILE" >"$tmp_file"
    mv "$tmp_file" "$ANCHORS_FILE"
else
    echo "Warning: jq not installed; could not append tx id to $ANCHORS_FILE automatically." >&2
fi

echo "Manifest generated: $manifest_path"
echo "Manifest SHA-256: $manifest_hash"
echo "Arweave/Irys tx id: $tx_id"
echo "Anchor registry file: $ANCHORS_FILE"
