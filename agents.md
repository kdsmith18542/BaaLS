# BaaLS — Agent Context

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

**Node 1** — primary validator
- Service: `baalsd.service`
- Binary: `/opt/baals/target/release/baalsd`
- Config: `/etc/baals/config.toml`
- Data: `/var/lib/baals`
- Ports: API 18080 (HTTPS), health 18082 (HTTP), WS 18081, P2P 9070
- Consensus key (env): `afd1fb8ddccc63ca29b2f11f371d9b959e1b07f217788ed04e2d1d18b50e7efd`
- Public key: `0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc`
- TLS: self-signed certs at `/etc/baals/tls/`

**Node 2** — second validator
- Service: `baalsd-node2.service`
- Config: `/etc/baals/config-node2.toml`
- Data: `/var/lib/baals-node2` (cloned from node 1 at height 7)
- Ports: API 18090 (HTTPS), health 18092 (HTTP), WS 18091, P2P 9071
- Consensus key (env): `4f1308bcfddd02537847297d92df8d4593e062a119bc8672e24b0e2a8d07743f`
- Public key: `002ec12b009aa6ba1790599675d066d0125c40683741d843eb43a0979678aaf1`
- Peers to: `127.0.0.1:9070` (node 1)

**Monitoring**
- Script: `/etc/baals/monitor.sh`
- Cron: `*/2 * * * *` — checks both nodes, logs to `/var/log/baals-monitor.log` + syslog
- Alerts on: node unreachable, unhealthy status, storage failure, height divergence > 5

### Build & Deploy

```bash
ssh grindsquad "source ~/.cargo/env && cd /opt/baals && git pull origin master && cargo build --release"
ssh grindsquad "systemctl restart baalsd && systemctl restart baalsd-node2"
```

Cargo requires `source ~/.cargo/env` on the VPS. Clear stale build artifacts with `rm -f target/release/deps/baals*` if feature flags change.

## Known Issues

### Critical

- **New nodes cannot sync from genesis.** Block replay fails because sender accounts don't exist in empty state. Node 2 was bootstrapped by cloning node 1's sled data. Any new validator must receive a state snapshot — there is no genesis state mechanism.

### Moderate

- **Stale authorized signers on node 1.** Keys `0dcc7432...` and `2ded0b...` from old test sessions are still authorized. The primary key also appears as a duplicate in the additional signers list. Clean up via `DELETE /api/v1/admin/signers/{hex}` (requires JWT + loopback).
- **Quorum threshold = 1.** Both nodes produce blocks independently. For real multi-validator consensus, bump `quorum_threshold` to 2 in both configs and enable round-robin.
- **Chain stalls without transactions.** `auto_block_mempool_threshold = 1` means no empty/heartbeat blocks. The chain goes silent between transactions.
- **P2P connection timeouts.** Inbound peer connections log `Connection timeout` after ~10s. Does not break sync (peers reconnect) but is noisy.
- **Admin endpoints need JWT for writes.** After the security fix in `1fde89d`, POST/DELETE on `/api/v1/admin/signers` requires a JWT obtained via `/auth/token`. Setup scripts must account for this.

## Remaining Work

### High Priority

1. **State snapshot / genesis sync** — let new nodes join the network without full block replay. Either implement a genesis state block that seeds initial accounts, or add a state-snapshot-at-height protocol.
2. **Clean up stale signers** — remove legacy test keys from node 1, deduplicate the primary key entry.
3. **Enable quorum threshold 2 + round-robin** — exercise real multi-validator consensus where both nodes must participate.

### Medium Priority

4. **Heartbeat / empty block production** — option to produce blocks on a timer even without transactions, preventing chain stall perception.
5. **Proper TLS certificates** — replace self-signed certs with Let's Encrypt or an internal CA.
6. **P2P timeout tuning** — increase connection read timeout or add keep-alive to reduce log noise.
7. **Monitor alerting** — add webhook or email notifications to `/etc/baals/monitor.sh`.
8. **Python SDK** — client library matching Rust's bincode transaction hashing (prototype exists in e2e test scripts).

### Lower Priority

9. RocksDB storage backend
10. Mobile SDKs (iOS/Android)
11. PoS / PoW / CRDT consensus plugins
12. Block explorer improvements

## Transaction Format Reference

Transaction hash (SHA-256) is computed over these fields in order:
1. `sender.as_bytes()` — raw 32 bytes
2. `nonce` — u64 LE
3. `timestamp` — u64 LE
4. `gas_limit` — u64 LE
5. `gas_price` — u64 LE
6. `priority` — u8
7. `chain_id` — u64 LE
8. `bincode(recipient)` — Address enum: `u32(variant) + u64(len) + bytes`
9. `bincode(payload)` — TransactionPayload enum
10. `bincode(metadata)` — if Some

Minimum gas limit: 21,000. Signature: Ed25519 over the 32-byte hash.

## Key Files

- `src/consensus.rs` — PoA validation, signing, round-robin, quorum checks
- `src/sync.rs` — P2P protocol, peer management, bootstrap peer resilience
- `src/runtime.rs` — block production, sync import, authorized signer management
- `src/ledger.rs` — account state, fee distribution, block application
- `src/cli/node.rs` — HTTP/WS/health servers, JWT auth, admin API routing
- `src/types.rs` — Block, Transaction, PublicKey, FeePolicy structs
- `src/config.rs` — TOML config parsing, fee/consensus/network settings
- `tests/cq_regression.rs` — fee system tests (4 modes + validation)
- `tests/multi_validator.rs` — multi-signer PoA tests
