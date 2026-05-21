# BaaLS - Agent Context

## Project Overview

BaaLS (Blockchain as a Local Service) is a Rust embedded blockchain node with:
- Ed25519 PoA consensus (multi-validator, round-robin capable)
- Dual storage backends: sled (default) and redb
- WASM smart contracts via wasmtime
- P2P sync with TLS (rustls + ring), mutual authentication, bootstrap peer resilience
- Fee system: None / Metered / Economic modes with operator/treasury/burn split
- JWT-authenticated admin API (loopback-only for writes)
- CLI binary: `baalsd`

## Infrastructure

### VPS (198.71.49.148, SSH alias: `grindsquad`)

**Node 1** - primary validator
- Service: `baalsd.service`
- Binary: `/opt/baals/target/release/baalsd`
- Config: `/etc/baals/config.toml`
- Data: `/var/lib/baals`
- Ports: API 18080 (HTTPS), health 18082 (HTTP), WS 18081, P2P 9070
- Consensus key (env): `afd1fb8ddccc63ca29b2f11f371d9b959e1b07f217788ed04e2d1d18b50e7efd`
- Public key: `0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc`
- TLS: internal CA signed cert chain at `/etc/baals/tls/server-chain.crt` with trust anchor `/etc/baals/tls/ca.crt`

**Node 2** - second validator
- Service: `baalsd-node2.service`
- Config: `/etc/baals/config-node2.toml`
- Data: `/var/lib/baals-node2` (historically cloned from node 1 at height 7)
- Ports: API 18090 (HTTPS), health 18092 (HTTP), WS 18091, P2P 9071
- Consensus key (env): `4f1308bcfddd02537847297d92df8d4593e062a119bc8672e24b0e2a8d07743f`
- Public key: `002ec12b009aa6ba1790599675d066d0125c40683741d843eb43a0979678aaf1`
- Peers to: `127.0.0.1:9070` (node 1)

**Monitoring**
- Script: `/etc/baals/monitor.sh`
- Cron: `*/2 * * * *` - checks both nodes, logs to `/var/log/baals-monitor.log` + syslog
- Alerts on: node unreachable, unhealthy status, storage failure, height divergence > 5

### Build & Deploy

```bash
ssh grindsquad "source ~/.cargo/env && cd /opt/baals && git pull origin master && cargo build --release"
ssh grindsquad "systemctl restart baalsd && systemctl restart baalsd-node2"
```

Cargo requires `source ~/.cargo/env` on the VPS. Clear stale build artifacts with `rm -f target/release/deps/baals*` if feature flags change.

## Recently Completed (2026-05-21)

- **State snapshot bootstrap sync is implemented and deployed.** Empty new nodes now request `GetSnapshot`, restore state, and converge without manual sled cloning.
- **Chain-state refresh after sync is implemented.** Runtime now updates in-memory chain state after successful peer sync imports.
- **Quorum + round-robin are enabled on both VPS nodes.** `/etc/baals/config.toml` and `/etc/baals/config-node2.toml` set `quorum_threshold = 2` and `round_robin = true`.
- **Signer sets are clean.** Node 1 and Node 2 signer lists include only each other; stale historical keys are removed.
- **Optional empty block production is implemented.** `consensus.produce_empty_blocks` config flag exists (default `false`).
- **Monitor alerting is implemented.** `/etc/baals/monitor.sh` now supports webhook/email notifications, cooldown-based alert suppression, and recovery notifications.
- **P2P timeout tuning is implemented.** `network.connection_timeout_ms` is now wired into `CustomSync`, and expected idle disconnects are logged at debug level instead of error.
- **TLS hardening is implemented.** Both nodes now run CA-signed certs with configured trust pinning (`tls_ca_cert_path = "/etc/baals/tls/ca.crt"`).
- **Empty-block policy was re-enabled safely.** With quorum signature collection + proposer gating, `produce_empty_blocks=true` is now enabled on both VPS nodes and chain height advances while converging.
- **Heartbeat safety guard is implemented in code.** Empty block production is now proposer-gated in round-robin mode (deterministic signer ordering), and empty heartbeat blocks are intentionally suppressed when `quorum_threshold > 1` to avoid timed same-height fork churn.
- **Quorum signature collection is implemented.** Proposers now request and attach peer quorum signatures over P2P before applying/broadcasting blocks when `quorum_threshold > 1`, and imported quorum-signed blocks validate correctly.
- **Quorum heartbeat path is now enabled and tested.** With reachable peers, `produce_empty_blocks=true` + `quorum_threshold > 1` + `round_robin=true` now produces converged empty blocks under quorum signatures; without quorum peers, heartbeat proposals are skipped.

## Known Issues

### Moderate

- **Heartbeat with quorum requires healthy peer quorum.** If quorum peers are unreachable, empty heartbeat proposals are skipped until signatures are available.
- **Transient one-block lag/reorg warnings may appear.** Nodes can briefly differ by one height before converging to the same tip under active heartbeat traffic.
- **P2P connection churn still occurs.** Peers may still close/reconnect on timeout windows, but expected idle timeout events are now debug-level rather than error-level.
- **Admin endpoints need JWT for writes.** POST/DELETE on `/api/v1/admin/signers` requires a token from `/auth/token`.

## Remaining Work

### Medium Priority

1. **Python SDK** - client library matching Rust's bincode transaction hashing (prototype exists in e2e test scripts).

### Lower Priority

3. RocksDB storage backend
4. Mobile SDKs (iOS/Android)
5. PoS / PoW / CRDT consensus plugins
6. Block explorer improvements

## Transaction Format Reference

Transaction hash (SHA-256) is computed over these fields in order:
1. `sender.as_bytes()` - raw 32 bytes
2. `nonce` - u64 LE
3. `timestamp` - u64 LE
4. `gas_limit` - u64 LE
5. `gas_price` - u64 LE
6. `priority` - u8
7. `chain_id` - u64 LE
8. `bincode(recipient)` - Address enum: `u32(variant) + u64(len) + bytes`
9. `bincode(payload)` - TransactionPayload enum
10. `bincode(metadata)` - if Some

Minimum gas limit: 21,000. Signature: Ed25519 over the 32-byte hash.

## Key Files

- `src/consensus.rs` - PoA validation, signing, round-robin, quorum checks
- `src/sync.rs` - P2P protocol, peer management, snapshot/bootstrap sync
- `src/runtime.rs` - block production, sync import, authorized signer management
- `src/ledger.rs` - account state, fee distribution, block application
- `src/cli/node.rs` - HTTP/WS/health servers, JWT auth, admin API routing
- `src/types.rs` - Block, Transaction, PublicKey, FeePolicy structs
- `src/config.rs` - TOML config parsing, fee/consensus/network settings
- `tests/cq_regression.rs` - fee system tests (4 modes + validation)
- `tests/multi_validator.rs` - multi-signer PoA tests
- `tests/integration.rs` - includes `test_state_snapshot_sync`, `test_empty_block_production`, `test_p2p_quorum_signature_collection`, and quorum heartbeat convergence tests

## Monitor Alerting Notes

- Script path: `/etc/baals/monitor.sh` (cron: `*/2 * * * *`)
- Versioned source: `scripts/monitor.sh`
- Config example: `scripts/baals-monitor.env.example`
- Optional config file: `/etc/default/baals-monitor` (override via `BAALS_MONITOR_CONFIG_FILE`)
- Optional environment variables:
  - `BAALS_MONITOR_WEBHOOK_URL` - webhook endpoint for alert/recovery JSON payloads
  - `BAALS_MONITOR_EMAIL_TO` - destination email address (requires `mail` or `mailx`)
  - `BAALS_MONITOR_ALERT_COOLDOWN_SECONDS` - suppression window for repeated alerts (default `900`)
  - `BAALS_MONITOR_STATE_DIR` - alert state directory (default `/var/lib/baals-monitor`)
