# Spec Compliance Notes

Intentional design deviations from the spec documents in `docs/`, with rationale.

## Resolved Gaps (2026-05-06 update)

| Gap | Status | Notes |
|-----|--------|-------|
| Block hash uses tx Merkle root | Resolved | `Block::calculate_hash()` now computes MerkleTree from tx hashes instead of serializing raw transactions. |
| ContractDeploy missing init_payload | Resolved | Added `init_payload: Option<Vec<u8>>` to `TransactionPayload::ContractDeploy`. |
| WASI block context always zero | Resolved | Added `block_index` and `block_timestamp` params to `ContractEngine::call_contract()` and `query_contract()`. Ledger passes real block context; runtime/CLI pass (0,0). |
| baals_call_contract missing value param | Resolved | Added `value: i64` (7th parameter) to WASI `baals_call_contract` host function. Inter-contract call tuples now include value. |
| Config not wired to consensus | Resolved | `build_runtime()` now accepts `block_time_ms` parameter passed from config; `block_time_ms` applied to `PoAConsensus::new()` and auto-block interval. |
| Block overflow tx allowed | Resolved | Removed "allow at least one" override; tx exceeding block gas/size limits is now properly rejected. |
| Auto-block threshold bypass | Resolved | Removed `|| !is_empty()` fallback; blocks only auto-produce when mempool reaches configured threshold. |
| Missing wallet delete CLI | Resolved | Added `WalletCommands::Delete` variant and handler using `Keystore::delete_key()`. |
| Config::set() missing keys | Resolved | Added `storage.cache_size_mb`, `storage.compression`, `storage.backend`, `network.tls_enabled`, `network.tls_cert_path`, `network.tls_key_path`, `network.tls_ca_cert_path`. |

## Type Deviations

### L1: Crate named `baals` not `libchain`
**Spec**: `BaaLS_Core_Engine_Runtime.md:243` — "Built as a Rust crate (libchain)"
**Actual**: `Cargo.toml` → `name = "baals"`
**Rationale**: Simpler, more recognizable name. The spec's `libchain` was a placeholder. All imports use `baals::*`.

### L2: Hash fields use `[u8; 32]` not `String`
**Spec**: `BaaLS_Core_Engine_Runtime.md:91-94` — `prev_hash: String`, `hash: String`
**Actual**: `src/types.rs:342-343` — `pub prev_hash: [u8; 32]`, `pub hash: [u8; 32]`
**Rationale**: Fixed-size byte arrays are more type-safe, avoid hex encoding/decoding bugs, and have deterministic memory layout. Display as hex via `format_hex()` helper. This is a deliberate improvement over the spec.

### L3: Metadata uses `BTreeMap<String, String>` not `Map<String, Value>`
**Spec**: `BaaLS_Core_Engine_Runtime.md:101,115` — `metadata: Option<Map<String, Value>>`
**Actual**: `src/types.rs:346,360` — `pub metadata: Option<BTreeMap<String, String>>`
**Rationale**: `serde_json::Value` adds a heavy dependency and permits arbitrary nesting. Flat `BTreeMap<String, String>` is simpler, deterministic (ordered keys), and sufficient for all metadata use cases (signer, signature, signed_at). Complex structures can serialize to strings.

### L4: `recipient` is `Address` enum not `PublicKey`
**Spec**: `BaaLS_Core_Engine_Runtime.md:107` — `recipient: PublicKey`
**Actual**: `src/types.rs:355` — `pub recipient: Address` (Wallet(PublicKey) | Contract(ContractId))
**Rationale**: Transactions can target both wallet accounts and smart contracts. The `Address` enum makes this explicit and type-safe. This is an improvement over the spec.

## API Deviations

### L7: `Runtime::start()` doesn't begin block production by default
**Spec**: `BaaLS_CLI_SDK_Wiring_Overview.md:31` — "Starts a BaaLS node... Runs in foreground"
**Actual**: `start()` sets is_running flag. Auto-block production is opt-in via `auto_block_interval_ms > 0`.
**Rationale**: Tests and embedded SDK users need control over when blocks are produced. The CLI `node start` enables auto-block by setting `auto_block_interval_ms = 5000` in `build_runtime()`.
**Enable**: Set `runtime.auto_block_interval_ms = 5000` before calling `start()`.

### L8: Single-validator PoA only (no multi-validator)
**Spec**: `BaaLS_Consensus_Engine_PoA.md:154-168` — Multi-validator, PoS, PoW, CRDT plugins
**Actual**: Only `PoAConsensus` with single authority. Other consensus types not implemented.
**Rationale**: MVP scope. The `ConsensusEngine` trait makes future plugins possible. Multi-validator PoA can be added by extending `PoAConsensus` to accept a validator set.

### L9: Keystore API takes `Option<PathBuf>` not required string
**Spec**: `BaaLS_Production_Readiness_Guide.md:25-31` — `Keystore::new("./keys")`
**Actual**: `Keystore::new(base_dir: Option<PathBuf>)` — `None` defaults to `~/.baals/keys/`
**Rationale**: More ergonomic API. Default path avoids hardcoding. CLI can override with `--keystore-dir`.

## Contract Engine Deviations

### M2: ContractEngine trait signatures differ from spec
**Spec**: `BaaLS_Core_Engine_Runtime.md:171-173`
- `deploy_contract(deployer, wasm_bytes, init_payload, storage) -> ContractId`
- `execute_contract_call(contract_id, sender, payload, storage) -> ContractExecutionResult`

**Actual**: `src/contracts.rs:47-65`
- `deploy_contract(deployer, deployer_nonce, wasm_bytes, init_payload, storage, gas_limit) -> ContractId`
- `call_contract(caller, contract_id, method_name, args, value, storage) -> Vec<u8>`
- `query_contract(contract_id, method_name, payload, storage) -> Vec<u8>`

**Rationale**: Added `deployer_nonce` for deterministic contract IDs, `gas_limit` for metering, `value` for native token transfer, and separate `method_name`/`args` instead of opaque `payload`. These are functional improvements. Plug-in implementors should match the actual trait.

### M3: No WasmRuntime sub-trait
**Spec**: `BaaLS_Core_Engine_Runtime.md:181-186` — `pub trait WasmRuntime`
**Actual**: WASM execution is handled inline in `BaaLSContractEngine::execute_wasm_contract()`. No separate trait.
**Rationale**: In practice, contract engines need deep integration with wasmtime's Module/Store/Linker types. A separate `WasmRuntime` trait would need to expose wasmtime internals, making it leaky. The `ContractEngine` trait provides the right abstraction level.

## Inter-Contract Call Limitations

### M10: Inter-contract call results not persisted between execution contexts
**Spec**: `BaaLS_Smart_Contract_Module.md:138-140` — synchronous inline calls
**Actual**: Inter-contract calls are processed asynchronously after the initial WASM execution returns. Results are available via `baals_read_call_result()` within the same execution chain but not across separate `execute_wasm_contract()` invocations.
**Rationale**: True synchronous inter-contract calls require re-entering the wasmtime execution during host function handlers, which is architecturally complex. Current implementation supports the pattern where Contract A initiates calls to Contract B and reads results in a subsequent invocation.

### L6: baals_storage_remove now properly deletes
**Resolved**: Previously inserted an empty vec instead of removing the key. Now calls `contract_storage.remove(&key)` and tracks deletions in `HostState.deleted_keys` for persistence.

## Remaining Known Gaps

All gaps below are tracked in `plan.md` with specific phases for resolution.

| Gap | Phase | Difficulty | Notes |
|-----|-------|------------|-------|
| M3: No WasmRuntime sub-trait | 23.4 | Easy | WASM execution inline in BaaLSContractEngine. Extract trait. |
| M5: Storage key prefixes | 22.1 | Medium | Raw keys used; add `"block:"`, `"acc:"`, `"code:"` prefixes with migration. |
| M6: get_transaction_by_id return type | 22.2 | Medium | Returns Transaction only; needs reverse tx-to-block index for (Block, Transaction). |
| M7: Real storage compaction | Deferred | Hard | Sled lacks compaction API; migration to redb/rocksdb would provide this. |
| M10: Persist inter-contract results | 23.1 | Medium | Results lost between separate contexts; engine-level result cache needed. |
| L5: NodeJS native addon | 26.1 | Hard | TypeScript types only. Requires napi-rs. |
| L8: Multi-validator PoA | Deferred | Medium | Only single authority. Extend PoAConsensus to accept validator set. |
| Sync layer never active | 24.1 | Hard | NoopSync always used; activate CustomSync behind --peer flag. |
| Fork resolution not wired | 24.2 | Medium | `resolve_fork()` exists but never called by runtime. |
| No peer discovery | 24.3 | Medium | mDNS/manual peer list; add `mdns-sd` crate. |
| No mempool persistence | 25.1 | Easy | Save/reload pending txs via storage trait. |
| No backup/restore CLI | 25.2 | Easy | Wrap existing `backup_to()`/`restore_from()` storage methods. |
| ContractCall.args is Vec<u8> | 23.2 | Medium | Spec says Vec<Vec<u8>> for structured ABI args. |
| Reentrancy bypass (inter-contract) | 23.3 | Medium | Fresh HostState per call; share executing_contracts guard. |
| MerkleTree vs SparseMerkleTree | 22.5 | Medium | Contract storage root uses MerkleTree; spec says SMT. |
| RedbStorage missing indexes | 22.4 | Medium | Height/address/contract/tx-count indexes missing in redb backend. |
| get_transactions_by_block unsorted | 22.3 | Easy | Return order not guaranteed; add explicit sort. |
| ContractDeployerAddress not validated | 23.6 | Easy | No reserved deployer addr convention enforced. |
| BaaLSContractEngine::new() dead param | 23.5 | Easy | Takes _storage but stores PhantomData only. |
| Go SDK (pure Go, not CGo) | 26.2 | Hard | Current Go bindings are CGo; need idiomatic Go SDK. |

### Resolved Gaps (previously in this section)

| Gap | Status |
|-----|--------|
| H6: Chain reorganization | ✅ Implemented — `reorganize_chain()` in runtime.rs |
| H10: Capability-based WASI security | ✅ Implemented — `ContractPermissions` checked in storage_write, call_contract, emit_event |
