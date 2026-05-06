# Spec Compliance Notes

Intentional design deviations from the spec documents in `docs/`, with rationale.

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

| Gap | Status | Notes |
|-----|--------|-------|
| H6: Chain reorganization | Deferred | Fork detection exists (`sync.rs`), no chain-switch logic |
| H10: Capability-based WASI security | Deferred | All contracts have equal host function access |
| M5: Storage key prefixes | Deferred | Raw keys used; migration risk if adding prefixes |
| M6: get_transaction_by_id return type | Deferred | Returns Transaction only, spec wants (Block, Transaction) |
| M7: Real storage compaction | Deferred | Sled lacks compaction API |
| L5: NodeJS native addon | Deferred | TypeScript types only, no napi-rs/neon implementation |
