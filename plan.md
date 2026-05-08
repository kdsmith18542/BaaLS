# BaaLS Production Plan

## Status: Pre-production hardening in progress

| Metric | Value |
|---|---|
| Spec compliance | 12 gaps resolved, 10 WONTFIX deviations |
| Safety criticals (P0) | Resolved |
| Security hardening (P1) | Resolved |
| Build | `cargo check` clean, 14 warnings |
| Cargo.lock cranelift | 0.130.2 (AIKIDO-2025-10778 — already patched) |

---

## Phase A: Remaining Production Hardening (P1/P2)

### A1. Gas Pricing
`src/ledger.rs:203` — `total_fee` hardcoded to `0u64`. Transactions pay no gas.
- Add `gas_price: u64` to `Transaction` struct and all 69+ construction sites
- Deduct `gas_price * gas_used` from sender balance
- Add `baalsd tx estimate-fee` CLI command

### A2. Per-sender time-windowed rate limiting
Mempool has count-based cap (100 tx/sender). No time-windowed limit.
- Add `max_tx_per_sender_per_second` to Mempool config
- Track per-sender submission timestamps

### A3. Expand fuzz targets
3 targets exist (deserialize + hash/verify). Missing:
- `fuzz_ledger_state_transition` — apply arbitrary txs to storage
- `fuzz_contract_execution` — WASM with hostile host function calls
- `fuzz_sync_messages` — deserialize all P2P message types
- `fuzz_merkle_proof` — verify proofs with tampered data

### A4. Release infrastructure
- Add `justfile` (dev command shortcuts: `just ci`, `just release`, `just run`)
- Reproducible build notes (`Cargo.lock` committed, `--locked` flag)
- `cargo package` / `cargo publish` readiness
- Binary checksums in releases

### A5. Keystore KDF upgrade
PBKDF2-SHA256 at 600k iterations is good but not gold-standard.
- Migration path to Argon2id or scrypt
- Encrypted keystore format versioning for forward compatibility

---

## Phase B: CLI Completion

### B1. Missing wallet commands
```
baalsd wallet show <id>
baalsd wallet verify <id> <message> <signature>
baalsd wallet change-password <id>
baalsd wallet export-public <id>
baalsd wallet recover --mnemonic "<words>"
```

### B2. Missing tx commands
```
baalsd tx sign --file unsigned_tx.json --wallet alice --out signed_tx.json
baalsd tx submit --file signed_tx.json --data-dir ./data
baalsd tx estimate-fee --file unsigned_tx.json
baalsd tx validate --file signed_tx.json
baalsd tx decode --file signed_tx.json
```

### B3. Missing query commands
```
baalsd query blocks --from 0 --to 100
baalsd query txs-by-account <pubkey> --limit 50
baalsd query txs-by-contract <contract_id> --limit 50
baalsd query mempool
baalsd query proof account <pubkey>
baalsd query proof contract <id> <key>
```

### B4. Missing db commands
```
baalsd db compact
baalsd db backup --output backup.baals
baalsd db restore --input backup.baals
baalsd db check-indexes
baalsd db rebuild-indexes
baalsd db export --out export.json
baalsd db import --input export.json
```

### B5. Missing dev commands
```
baalsd dev dump-state --out state.json
baalsd dev replay-block <height>
baalsd dev replay-chain
baalsd dev fuzz-wasm --wasm contract.wasm
baalsd dev inspect-wasm contract.wasm
baalsd dev verify-merkle-root
baalsd dev repair-indexes
```

### B6. New top-level command groups
```
baalsd key        # low-level key tools (generate, inspect keypair)
baalsd api        # HTTP client wrapper (health, submit, deploy, call)
baalsd p2p        # peer management (peers, add-peer, remove-peer, ping, sync-now)
baalsd proof      # Merkle proof tools (account, contract, verify)
baalsd contract   # contract tooling (inspect, simulate, estimate-gas, abi, verify-wasm)
baalsd admin      # secured local admin ops (rotate-consensus-key, tls generate, token generate)
baalsd doctor     # environment/config diagnostics
```

---

## Phase C: Developer Experience

### C1. `justfile`
```
just check          cargo check --all-targets --all-features
just fmt            cargo fmt --all
just fmt-check      cargo fmt --all -- --check
just clippy         cargo clippy --all-targets --all-features -- -D warnings
just test           cargo test --all-features
just test-release   cargo test --release --all-features
just audit          cargo audit
just deny           cargo deny check
just bench          cargo bench
just docs           cargo doc --no-deps --all-features
just ci             fmt-check + clippy + test + audit + deny
just build          cargo build
just release        cargo build --release --locked
just run            cargo run -- node start --data-dir ./data
just clean-data     rm -rf ./data
```

### C2. Production build flags
- `--locked` in CI release builds
- `cargo miri test` for unsafe code audit (nightly)
- `cargo outdated` / `cargo machete` for dependency hygiene

---

## Phase D: Documentation

### D1. Replace Spec_Compliance_Notes.md
Replace the contradictory "All Gaps Resolved" doc with:
- `docs/COMPLIANCE.md` — spec gap tracker with `RESOLVED` / `WONTFIX` / `OPEN` status per item
- Auto-generated from CI results (test count, clippy status, audit status, fuzz status)

### D2. Operator documentation
- `docs/OPERATING.md` — node deployment, backup/restore, monitoring, key management
- `docs/TLS_GUIDE.md` — TLS cert generation, pinning, mTLS setup

---

## Target CLI Layout (end state)

```
baalsd
  node    start, stop, status, backup, restore
          config init, set, get
  wallet  create, list, show, import, export, export-public
          sign, verify, delete, rotate, change-password, recover
  tx      transfer, data, deploy-contract, call-contract
          sign, submit, inspect, validate, estimate-fee, decode
  query   head, block, tx, account, contract-state, contract-call
          mempool, blocks, txs-by-account, txs-by-contract
  contract inspect, simulate, estimate-gas, abi, verify-wasm
  proof   account, contract, verify
  p2p     peers, add-peer, remove-peer, ping, sync-now
  db      version, migrate, verify, compact, backup, restore
          check-indexes, rebuild-indexes, export, import
  dev     generate-keys, validate-tx, storage-stats, performance-report
          validate-chain, monitor, dump-state, replay-block, replay-chain
          fuzz-wasm, inspect-wasm, verify-merkle-root, repair-indexes
  admin   rotate-consensus-key, export-node-id, tls generate, tls fingerprint, token generate
  doctor
  key     generate, inspect, sign, verify

Global flags: --json, --verbose, --storage-backend [sled|redb], --data-dir <path>
```
