# Spec Compliance Notes

Last updated: 2026-05-19

This file records intentional, current design deviations from the specification documents under `docs/`.

## Current Evidence

- `cargo fmt --all -- --check` passes
- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo test --all-features` passes (77 passed, 0 failed, 1 ignored)

## Type / Data Model Deviations

### L1: Crate named `baals` instead of `libchain`
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: `libchain` was a placeholder name; `baals` is the shipped crate identity.

### L2: Hash fields use `[u8; 32]` instead of `String`
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: fixed-size bytes are safer and deterministic; hex is rendered at boundaries.

### L3: Metadata uses `BTreeMap<String, String>`
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: deterministic ordering and lower complexity versus nested JSON values.

### L4: Transaction recipient is `Address` enum
- Spec reference: `BaaLS_Core_Engine_Runtime.md`
- Rationale: explicit modeling of wallet vs contract recipients.

## Runtime / API Deviations

### L7: `Runtime::start()` is lifecycle-only by default
- Auto block production is opt-in via runtime config.
- Rationale: embedded/test use-cases need explicit control.

### L8: Partial multi-validator support
- Current state: authorized signer set persistence and rotation structures exist.
- Not yet implemented: quorum-threshold multi-validator finalization semantics.
- Rationale: staged rollout; signer-set support exists, full quorum protocol is deferred.

### L9: Keystore constructor accepts optional base dir
- Current API: `Keystore::new(Option<PathBuf>)`
- Rationale: ergonomic default path behavior for CLI and embedded SDK use.

## Consensus / Network Scope Notes

### L10: WebSocket push streams are not implemented
- REST API is implemented and canonicalized under `/api/v1/*`.
- WebSocket streaming remains planned.

### L11: P2P command group wiring is partial
- Some CLI subcommands are explicitly marked not implemented to avoid misleading success output.

## Historical Resolutions (Retained)

The following major historical gaps are resolved in the current tree:

- Block hash uses transaction Merkle root
- Contract deploy supports `init_payload`
- WASI block context parameters are wired
- `baals_call_contract` includes value parameter
- Config block time is wired into consensus/runtime
- Block overflow transaction rejection is enforced
- Auto-block threshold bypass behavior removed
- Wallet delete CLI command exists
- Config key coverage for storage/network fields expanded

## Reproducibility

Use the following commands to validate the current compliance snapshot:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
