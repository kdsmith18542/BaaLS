# Operating BaaLS

## Deployment

### Prerequisites
- Rust 2021 edition (MSRV: current stable)
- 512 MB RAM minimum, 1 GB recommended
- 100 MB disk for binary + initial data

### Quick Start
```bash
# Build
cargo build --release --locked

# Generate config
./target/release/baalsd node config init --output baals.toml

# Create wallet
./target/release/baalsd wallet create --name operator

# Start node
./target/release/baalsd node start --data-dir ./data --port 8080
```

### Daemon Mode
```bash
# Start as background daemon
./target/release/baalsd node start --data-dir ./data --daemon

# Check status
./target/release/baalsd node status --data-dir ./data

# Stop
./target/release/baalsd node stop --data-dir ./data
```

## Backup and Restore

### Using db commands
```bash
# Backup
baalsd db backup --data-dir ./data --output backup.baals

# Restore to fresh directory
baalsd db restore --data-dir ./data --input backup.baals
```

### Using node commands
```bash
# Online backup (node must be stopped or use read-only mode)
baalsd node backup --data-dir ./data --output backup.baals

# Restore
baalsd node restore --data-dir ./data --input backup.baals
```

### Automated Backups
Set `backup_interval_secs` in config.toml to enable automatic periodic backups.

## Database Management

```bash
# Check version info
baalsd db version --data-dir ./data

# Verify integrity
baalsd db verify --data-dir ./data

# Compact storage (reclaim space)
baalsd db compact --data-dir ./data

# Check indexes
baalsd db check-indexes --data-dir ./data

# Rebuild indexes (if corruption detected)
baalsd db rebuild-indexes --data-dir ./data

# Migrate to latest schema
baalsd db migrate --data-dir ./data

# Dry run migration
baalsd db migrate --data-dir ./data --dry-run

# Export all accounts to JSON
baalsd db export --data-dir ./data --out export.json

# Import accounts from JSON
baalsd db import --data-dir ./data --input export.json
```

## Key Management

### Wallet Operations
```bash
# Create new wallet
baalsd wallet create --name alice

# List wallets
baalsd wallet list

# Show wallet details
baalsd wallet show <public_key>

# Export private key
baalsd wallet export <public_key>

# Export public key only
baalsd wallet export-public <public_key>

# Sign a message
baalsd wallet sign <public_key> "message"

# Verify a signature
baalsd wallet verify <public_key> "message" <signature_hex>

# Change password
baalsd wallet change-password <public_key>

# Recover from seed
baalsd wallet recover --seed-hex <32_byte_hex>

# Rotate key (re-encrypt with new password)
baalsd wallet rotate <public_key>
```

### Low-Level Key Tools
```bash
# Generate raw keypair
baalsd key generate

# Inspect key bytes
baalsd key inspect <hex>

# Sign with raw private key
baalsd key sign <private_key_hex> "message"

# Verify with public key
baalsd key verify <public_key_hex> "message" <signature_hex>
```

## Transaction Lifecycle

```bash
# 1. Create and sign a transfer transaction
# Create unsigned tx JSON, then:
baalsd tx sign --file unsigned.json --wallet <pubkey> --out signed.json

# 2. Validate locally before submission
baalsd tx validate --file signed.json

# 3. Estimate fees
baalsd tx estimate-fee --file signed.json --gas-price 1

# 4. Submit to node
baalsd tx submit --file signed.json --data-dir ./data

# 5. Inspect/decode
baalsd tx decode --file signed.json
baalsd tx inspect signed.bin
```

## Querying the Chain

```bash
# Latest block
baalsd query head --data-dir ./data

# Block by height or hash
baalsd query block 42 --data-dir ./data
baalsd query block <block_hash_hex> --data-dir ./data

# Range of blocks
baalsd query blocks --from 0 --to 100 --data-dir ./data

# Transaction by hash
baalsd query tx <tx_hash_hex> --data-dir ./data

# Total supply
baalsd query supply --data-dir ./data

# Account
baalsd query account <pubkey_hex> --data-dir ./data

# Transactions by account
baalsd query txs-by-account <pubkey_hex> --limit 50 --data-dir ./data

# Contract state
baalsd query contract-state --contract-id <cid_hex> --key <key_hex> --data-dir ./data

# Contract query (read-only call)
baalsd query contract-call --contract-id <cid_hex> --method "method" --args "args"

# Mempool
baalsd query mempool --data-dir ./data
```

## Merkle Proofs

```bash
# Generate account proof
baalsd proof account <pubkey_hex> --data-dir ./data

# Generate contract storage proof
baalsd proof contract <contract_id_hex> <key_hex> --data-dir ./data

# Verify a proof from JSON file
baalsd proof verify proof.json
```

## Development Tools

```bash
# Validate chain consistency
baalsd dev validate-chain --data-dir ./data

# Dump full state to JSON
baalsd dev dump-state --out state.json --data-dir ./data

# Replay block at height
baalsd dev replay-block 42 --data-dir ./data

# Replay entire chain
baalsd dev replay-chain --data-dir ./data

# Verify Merkle root matches
baalsd dev verify-merkle-root --data-dir ./data

# Fuzz/inspect WASM modules
baalsd dev fuzz-wasm --wasm contract.wasm
baalsd dev inspect-wasm --wasm contract.wasm

# Repair indexes
baalsd dev repair-indexes --data-dir ./data

# Storage statistics
baalsd dev storage-stats --data-dir ./data

# Performance report
baalsd dev performance-report --data-dir ./data

# Real-time monitor
baalsd dev monitor --data-dir ./data --detailed
```

## Diagnostics

```bash
# Environment checks
baalsd doctor
```

## Storage Backends

BaaLS supports two storage backends:

- **sled** (default): B-tree based, good general performance
- **redb**: Copy-on-write B-tree, better concurrent read performance

```bash
# Use redb
baalsd node start --storage-backend redb --data-dir ./redb-data
```

## Monitoring

Health endpoint: `http://localhost:8080/health`

Requires `BAALS_ADMIN_TOKEN` environment variable for mutating endpoints:
```bash
export BAALS_ADMIN_TOKEN="your-secret-token"
```

### HTTP API Endpoints

Canonical routes are under `/api/v1/*`. Legacy routes remain supported for compatibility.

| Canonical Endpoint | Legacy Alias | Method | Auth | Description |
|--------------------|--------------|--------|------|-------------|
| `/health` (or `/api/v1/health`) | - | GET | No | Node health status |
| `/api/v1/blocks/latest` | `/block/latest` | GET | No | Latest block info |
| `/api/v1/blocks/{h}` | `/block/by_height/{h}` | GET | No | Block by height |
| `/api/v1/blocks/hash/{hash}` | `/block/by_hash/{hash}` | GET | No | Block by hash |
| `/api/v1/accounts/{pk}` | - | GET | No | Query account state |
| `/api/v1/supply` | `/supply` | GET | No | Chain total supply snapshot |
| `/api/v1/transactions/{hash}` | `/tx/{hash}` | GET | No | Transaction details + status + finality |
| `/api/v1/transactions/{hash}/finality` | `/tx/{hash}/finality` | GET | No | Transaction confirmation/finality status |
| `/proof/account/{pk}` | - | GET | No | Account Merkle proof |
| `/proof/contract/{id}/storage/{key}` | - | GET | No | Contract storage proof |
| `/api/v1/contracts/call` | `/contract/query` | POST | No | Read-only contract call |
| `/api/v1/transactions` | `/tx/submit` | POST | Bearer | Submit transaction |
| `/api/v1/accounts` | `/account` | POST | Bearer | Create account |
| `/api/v1/contracts/deploy` | `/contract/deploy` | POST | Bearer | Deploy contract |
| `/api/v1/contracts/invoke` | `/contract/call` | POST | Bearer | State-changing contract call |
| `/metrics` | - | GET | No | Prometheus-style node metrics (loopback) |

## Configuration

Default config file: `config.toml`

Key settings:
```toml
[storage]
backend = "sled"           # sled or redb
cache_size_mb = 128
compression = false

[consensus]
block_time_ms = 5000       # Block production interval
finality_depth = 12        # Confirmations required for finality
max_reorg_depth = 50       # Reject forks that require rolling back deeper than this

[network]
tls_enabled = false
tls_cert_path = ""
tls_key_path = ""
tls_ca_cert_path = ""
```
