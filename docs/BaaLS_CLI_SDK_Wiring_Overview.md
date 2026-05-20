Deep Dive Blueprint: BaaLS CLI & SDK Wiring Overview
Purpose: To define the external programmatic and command-line interfaces that allow users and applications to interact with a BaaLS instance. This blueprint emphasizes ease of use, broad language support, and secure access to the core BaaLS Runtime functionalities.

Relationship to BaaLS Core:
Both the CLI and SDKs act as clients to the core baals crate and its Runtime module. They translate user commands or programmatic calls into specific function invocations within the BaaLS runtime, handling data serialization, deserialization, and error propagation across language boundaries where necessary.

Core Principles:

Ease of Use: Intuitive commands and clear APIs for both human and programmatic interaction.

Broad Language Support: Provide idiomatic SDKs for popular languages, leveraging FFI where native Rust integration isn't feasible.

Comprehensive Functionality: Expose all core BaaLS features, from node management to smart contract interaction.

Secure Access: Ensure private keys are handled securely and interactions with the BaaLS instance (especially for networked scenarios) can be authenticated.

Structured Output: Provide machine-readable (JSON) output options for easy integration into scripts and other applications.

1. CLI Tools (baalsd executable)
The baalsd command-line interface will be the primary tool for developers to interact with a BaaLS instance directly, manage its lifecycle, and perform quick operations. Built using a robust Rust CLI framework like clap.

Role: Direct interaction, scripting, node administration, development, and debugging.

Key Command Categories & Examples:

Node Management (baalsd node ...): For starting, stopping, and configuring a BaaLS instance.

baalsd node start [path/to/config.toml]

Starts a BaaLS node, optionally using a specified configuration file. Runs in foreground or daemonized.

baalsd node stop

Gracefully stops a running BaaLS node.

baalsd node status

Displays the current state of the node (running/stopped, latest block, mempool size, sync status).

baalsd node config init

Generates a default config.toml file.

baalsd node config set <key> <value>

Updates a specific configuration parameter.

Wallet Management (baalsd wallet ...): For keypair generation, import, and listing.

baalsd wallet create [--name <name>]

Generates a new cryptographic keypair and stores it securely (e.g., encrypted local keystore). Returns public key.

baalsd wallet list

Lists all managed public keys (addresses).

baalsd wallet import <private_key_hex>

Imports an existing private key.

baalsd wallet export <public_key> [--password <password>]

Exports an encrypted private key (with passphrase).

baalsd wallet sign <public_key> <message_hex>

Signs a raw message with a specified key.

Transaction Submission (baalsd tx ...): For constructing and submitting various transaction types.

baalsd tx transfer --sender <pubkey> --recipient <address> --amount <value> [--memo <string>]

Submits a native token transfer transaction.

baalsd tx deploy-contract --sender <pubkey> --wasm <path/to/wasm> [--init-args <json>] [--gas-limit <units>]

Deploys a WASM smart contract. The init-args JSON would be passed to the contract's initialization function.

baalsd tx call-contract --sender <pubkey> --contract-id <id> --method <name> --args <json> [--value <amount>] [--gas-limit <units>]

Calls a function on a deployed smart contract. args would be JSON and translated to the contract's expected ABI encoding. value for native token transfer with call.

baalsd tx data --sender <pubkey> --data <hex_or_string>

Submits a raw data transaction.

baalsd tx inspect <path/to/signed_tx_file>

Parses and displays details of a raw signed transaction file.

Chain Query (baalsd query ...): For retrieving data from the blockchain.

baalsd query head

Displays the latest block hash and height.

baalsd query block <hash_or_height>

Retrieves and displays a specific block's details (full content or summary).

baalsd query tx <tx_hash>

Retrieves and displays a specific transaction's details.

baalsd query account <address>

Displays the details of a wallet or contract account (balance, nonce, contract code hash).

baalsd query contract-state --contract-id <id> --key <hex_key>

Reads a specific key-value from a contract's local storage.

baalsd query contract-call --contract-id <id> --method <name> --args <json>

Executes a read-only query (simulated call) on a smart contract without changing state.

Development/Debugging (baalsd dev ...): Utilities for testing and development.

baalsd dev generate-keys [--count <n>]

Generates new keypairs for testing purposes.

baalsd dev simulate-contract --wasm <path/to/wasm> --method <name> --args <json> [--sender <pubkey>]

Runs a WASM module locally in the sandbox for testing without deploying to the chain. Displays output, gas usage, and simulated state changes.

baalsd dev validate-tx <path/to/raw_tx_file>

Validates a raw transaction file for format and signature.

Output Formats:

Default: Human-readable, nicely formatted tables or descriptive text.

--json flag: Output machine-readable JSON for all queries, enabling easy piping to other tools.

Configuration: CLI commands will use a hierarchical configuration system (CLI flags > Environment Variables > Config File > Default values).

2. SDKs (Software Development Kits)

SDKs provide the necessary libraries and tools for developers to integrate BaaLS functionalities directly into their applications (desktop, mobile, web backend).

### 2.1 Rust SDK (libbaals)

The Rust SDK is the canonical interface to BaaLS, providing direct access to the Runtime and all core features.

**Availability:**
- Published on crates.io as `baals`
- Library crate with public API for programmatic access
- Feature-gated optional dependencies (mdns, profiling, etc.)

**Core API (libbaals/src/lib.rs):**

```rust
pub struct Runtime { ... }

impl Runtime {
    pub fn new(config: RuntimeConfig) -> Result<Self>;
    pub fn apply_transaction(&mut self, tx: Transaction) -> Result<Receipt>;
    pub fn apply_block(&mut self, block: Block) -> Result<BlockReceipt>;
    pub fn query_account(&self, address: Address) -> Result<Account>;
    pub fn query_contract_state(&self, contract_id: [u8; 32], key: &[u8]) -> Result<Vec<u8>>;
    pub fn generate_signing_key() -> Result<SigningKey>;
    pub fn get_chain_state(&self) -> ChainState;
}

pub struct Keystore { ... }

impl Keystore {
    pub fn new(path: &str) -> Result<Self>;
    pub fn create_wallet(&mut self, name: &str) -> Result<PublicKey>;
    pub fn sign(&self, public_key: &PublicKey, message: &[u8]) -> Result<Signature>;
    pub fn list_keys(&self) -> Vec<PublicKey>;
}
```

**Usage Example:**

```rust
use baals::{Runtime, RuntimeConfig, Keystore, Transaction, Address};

let mut runtime = Runtime::new(RuntimeConfig::default())?;
let mut keystore = Keystore::new("./keys")?;

let alice = keystore.create_wallet("alice")?;
let tx = Transaction::transfer(alice, Address::zero(), 1000)?;
let receipt = runtime.apply_transaction(tx)?;
```

### 2.2 JavaScript/TypeScript SDK (baals-js)

Provides Node.js bindings. Current repository state includes two beta tracks.

**Availability:**
- Native addon path (`napi-rs`): `sdk/nodejs-native` (package metadata `@baals/sdk`)
- Legacy FFI wrapper path (`ffi-napi`): `sdk/nodejs` (package metadata `@baals/sdk-ffi`)
- TypeScript definition files are present for both SDK directories
- Browser/WASM packaging is planned, not shipped in this repo

**API:**

```typescript
import { Runtime, Keystore, Transaction } from '@baals/sdk';

const runtime = new Runtime({ dataDir: './data' });
const keystore = new Keystore('./keys');

const alice = await keystore.createWallet('alice');
const tx = Transaction.transfer(alice, Address.ZERO, 1000n);
const receipt = await runtime.applyTransaction(tx);
```

**Features:**
- Promise-based async API
- Full TypeScript support
- Event emitters for block/transaction notifications
- JSON serialization of all types
- Error handling with descriptive messages

### 2.3 Python SDK (baals-py)

Status: **Planned, not implemented in this repository**.

### 2.4 Go SDK (baals-go)

Provides idiomatic Go bindings via cgo and FFI.

**Availability:**
- Go package/module path: `github.com/baals/sdk`
- Pure Go client library (no required CGO linkage to Rust runtime)
- Support for Go 1.19+

**API:**

```go
import "github.com/baals/sdk"

runtime, _ := baals.NewRuntime(config)
keystore, _ := baals.NewKeystore("./keys")

alice, _ := keystore.CreateWallet("alice")
tx := baals.NewTransfer(alice, baals.ZeroAddress, 1000)
receipt, _ := runtime.ApplyTransaction(tx)
```

### 2.5 HTTP/REST API (via API Server)

For remote access, BaaLS exposes a REST API via the HTTP server (port 8080 by default).

**Endpoints:**

```
POST   /api/v1/transactions        Submit a signed transaction
GET    /api/v1/blocks/<height>     Fetch block by height
GET    /api/v1/accounts/<addr>     Query account state
POST   /api/v1/contracts/call      Execute read-only contract call
GET    /api/v1/blocks/latest       Fetch latest block summary
GET    /health (or /api/v1/health) Health check
```

**Authentication:**
- Required for mutating endpoints: short-lived Bearer JWT from `POST /auth/token`
- Mutating endpoints are restricted to loopback clients
- Read-only endpoints do not require a token
- Rate limiting is enforced per remote IP address

### 2.6 WebSocket API (Streaming)

Status: **Planned, not implemented in the current codebase**.

For real-time updates, clients will eventually support WebSocket streams.

**Endpoints:**

```
ws://localhost:8080/ws/blocks      Stream new blocks
ws://localhost:8080/ws/transactions Stream new transactions
```

**Message Format:**

```json
{
  "type": "block",
  "data": { "height": 100, "hash": "0x...", ... }
}
```

### 2.7 FFI / C Bindings

For languages without native SDKs (C, C++, Ruby, etc.), BaaLS exports a stable C API via `libbaals_ffi.so`/`.dll`.

**Header:**

```c
// libbaals_ffi.h
typedef struct BaalsRuntime BaalsRuntime;

BaalsRuntime* baals_runtime_new(const char* config_path);
int baals_apply_transaction(BaalsRuntime* rt, const uint8_t* tx_bytes, size_t tx_len);
void baals_runtime_free(BaalsRuntime* rt);
```

**Build:**

```bash
cargo build --release --features ffi
# Produces: target/release/libbaals_ffi.so (Linux), .dll (Windows), .dylib (macOS)
```

---

## 3. Security Best Practices

### For CLI Users
- Store keystore files in secure locations with restricted permissions (0600 on Unix)
- Use strong passphrases for encrypted keys
- Never share private keys or mnemonics
- Validate TLS certificates when connecting to remote nodes

### For SDK Users
- Never log private keys or signatures
- Validate input types and ranges before constructing transactions
- Use HTTPS for remote connections
- Implement retry logic with exponential backoff for transient failures
- Handle cryptographic errors gracefully

### For SDK Maintainers
- Audit FFI boundaries for memory safety
- Test against multiple language runtimes
- Document key rotation and backup procedures
- Provide security advisories via security@baals.dev
- Keep dependencies updated (use `cargo audit` / equivalent)

---

## 4. Configuration

### Environment Variables

Currently implemented environment variables in this repository:

| Variable | Purpose | Default |
|----------|---------|---------|
| `BAALS_CONFIG` | Config file path override | (none) |
| `BAALS_CONSENSUS_KEY` | Raw consensus private key (hex) | (none) |
| `BAALS_CONSENSUS_PASSWORD` | Keystore password for consensus key | (none) |
| `BAALS_BACKUP_KEY` | AES-256-GCM key for backup encryption | (none) |

### Config File Format

All SDKs support TOML configuration files:

```toml
[node]
data_dir = "./data"
port = 8080

[consensus]
block_time_ms = 5000
max_transactions_per_block = 1000

[logging]
level = "info"

[network]
max_peers = 50
connection_timeout_ms = 30000
```

---

## 5. Roadmap

### Current Support Matrix (2026-05-19)

| Area | Status | Notes |
|------|--------|-------|
| Rust SDK (`baals` crate) | GA | Canonical runtime API |
| HTTP REST API (`/api/v1/*` + legacy aliases) | GA | Mutating routes require JWT bearer token + loopback |
| CLI core runtime workflows | Beta | Core node/wallet/tx/query/db/proof/dev paths are implemented |
| CLI `p2p` group | Planned | Explicitly not wired to live sync state yet |
| CLI advanced `contract`/`admin` subcommands | Planned | Some subcommands intentionally return not implemented |
| Go SDK (`sdk/go`) | Beta | Pure-Go client wrapper around daemon/API flow |
| Node.js FFI SDK (`sdk/nodejs`) | Beta | Legacy ffi-napi package |
| Node.js native SDK (`sdk/nodejs-native`) | Beta | napi-rs addon path |
| Python SDK (`baals-py`) | Planned | Not present in this repository |
| WebSocket streaming API | Planned | Not implemented in current runtime |

### Near-term
- Complete runtime-backed `p2p` CLI actions
- Fill `contract`/`admin` command gaps
- Add WebSocket streaming endpoints

### Future
- Python SDK
- Mobile SDKs (Swift, Kotlin)
- Formal WASM security audit

