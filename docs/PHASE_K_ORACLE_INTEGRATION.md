# Phase K — Resurgence Protocol Oracle Integration

**Status (2026-05-25):** Live REST oracle path is active in production-like use. Dedicated WASM oracle contract remains optional future hardening.

## Overview

Phase K integrates BaaLS as the immutable attestation registry between ChronoNode (dormancy detection on UTXO chains) and the Resurgence EVM contracts (reward minting on Arbitrum/Polygon).

**Flow:**
```
ChronoNode detects dormant address → generates DormancyProof
                ↓ (sign with ChronoNode ed25519 key)
BaaLS REST /api/v1/oracle/attest ← receives signed proof
                ↓ (verify ChronoNode signature, sign with BaaLS ed25519 key)
BaaLS storage ← records OracleAttestation
                ↓
EVM chain (Arbitrum/Polygon) ← EVMSubmitter posts signed attestation
                ↓
Resurgence RewardDistributor ← verifies BaaLS signature, mints RESURGE
```

## Configuration

### Basic Setup

```toml
# config.toml
[oracle]
enabled = true
evm_rpc = "https://arb-sepolia.g.alchemy.com/v2/YOUR_KEY"
reward_distributor = "0x1234567890123456789012345678901234567890"  # Arbitrum RewardDistributor address
amoy_rpc = "https://rpc-amoy.polygon.technology/"  # For spoke-chain verification
evm_chain_id = 421614  # Arbitrum Sepolia
evm_private_key_env = "BAALS_EVM_PRIVATE_KEY"  # Env var holding secp256k1 private key (hex)
```

### Environment Variables

```bash
# BaaLS node signing keys
export BAALS_CONSENSUS_PASSWORD="your_consensus_password"  # Unlocks consensus.key.enc
export BAALS_EVM_PASSWORD="your_evm_password"              # Unlocks evm.key.enc

# EVM chain credentials (used only if evm.key.enc doesn't exist)
export BAALS_EVM_PRIVATE_KEY="0xabcd..."  # secp256k1 private key for EVM signing
```

## Oracle Keypair Rotation

Operational runbook is now documented in:

- `docs/RELAY_ORACLE_KEY_ROTATION.md`

Key points:

- Oracle and relay use the same secp256k1 signer configured by `oracle.evm_private_key_env`.
- Role grants must be completed before cutover:
  - `DORMANCY_ORACLE_ROLE`
  - `RELAY_MINTER_ROLE`
- Consensus key rotation remains separate (`baalsd admin rotate-consensus-key`).

## REST API Reference

### K.2 — Submit Dormancy Attestation

**Endpoint:** `POST /api/v1/oracle/attest`

**Authentication:** No JWT required. Endpoint is loopback-restricted by node policy (ChronoNode should submit from localhost/co-located environment or approved local proxy).

**Request Body (DormancyProof):**
```json
{
  "version": "chrononode:dormancy:v1",
  "chain_id": "bitcoin",
  "address": "1A1z7agoat7qcUeF...",
  "dormant_since_block": 700000,
  "current_block": 850000,
  "threshold_blocks": 100000,
  "signer_pubkey": "abcd1234....",
  "signature": "1234abcd...."
}
```

**Response (OracleAttestation):**
```json
{
  "status": "ok",
  "attestation": {
    "proof": { /* original DormancyProof */ },
    "baals_pubkey": "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc",
    "baals_signature": "signature_bytes_hex",
    "attested_at_block": 1234567,
    "baals_block_hash": "0xabcd1234..."
  }
}
```

**Error Responses:**
- `400`: Invalid proof JSON, missing fields, or bad signature
- `500`: Internal error

### K.3 — Query Attestations

#### Query Specific Address

**Endpoint:** `GET /api/v1/oracle/attestations/{chain_id}/{address}`

**Response:**
```json
{
  "attestation": {
    "proof": { /* DormancyProof */ },
    "baals_pubkey": "...",
    "baals_signature": "...",
    "attested_at_block": 1234567,
    "baals_block_hash": "..."
  }
}
```

#### List All Attested Addresses for a Chain

**Endpoint:** `GET /api/v1/oracle/attestations/{chain_id}`

**Response:**
```json
{
  "chain_id": "bitcoin",
  "count": 42,
  "attestations": [
    { /* OracleAttestation 1 */ },
    { /* OracleAttestation 2 */ },
    ...
  ]
}
```

**Error Responses:**
- `404`: Chain not found or attestation not found

## ChronoNode Integration

### Setup

1. **Deploy ChronoNode** with dormancy detection enabled.
   - Monitors Bitcoin, Dogecoin, Ethereum, Monero addresses for dormancy.
   - Generates DormancyProof after threshold blocks with no activity.

2. **Configure ChronoNode Submitter** to POST proofs to BaaLS:
   ```yaml
   # chrononode/config.yaml
   oracle:
     baals_url: "http://127.0.0.1:18080"
     endpoint: "/api/v1/oracle/attest"
     retry_count: 5
     retry_backoff_seconds: 60
   ```

3. **ChronoNode signs each proof** with its ed25519 key before submitting.

4. **BaaLS verifies** the ChronoNode signature and counter-signs with its own key.

5. **OracleAttestation returned** to ChronoNode contains both signatures and chain anchor.

### Example: Bitcoin Address Dormancy

```bash
# ChronoNode detects address dormant for 100k blocks
curl -X POST http://127.0.0.1:18080/api/v1/oracle/attest \
  -H "Content-Type: application/json" \
  -d '{
    "version": "chrononode:dormancy:v1",
    "chain_id": "bitcoin",
    "address": "1A1z7agoat7qcUeF4AMpXapZEUVXKLatJ5",
    "dormant_since_block": 750000,
    "current_block": 850000,
    "threshold_blocks": 100000,
    "signer_pubkey": "abc123...",
    "signature": "def456..."
  }'

# Response contains BaaLS-signed attestation
{
  "status": "ok",
  "attestation": { ... }
}
```

## EVM Chain Setup

### Arbitrum Sepolia (Hub Chain)

1. **Deploy Resurgence RewardDistributor** with oracle-consumer logic:
   - Accepts `submitDormancyProof(...)` calls
   - Authorizes caller via `DORMANCY_ORACLE_ROLE`
   - Records dormancy proof, authorizes RESURGE minting

2. **Authorize BaaLS EVM submitter address**:
   ```solidity
   RewardDistributor.grantRole(DORMANCY_ORACLE_ROLE, 0x...baals_evm_address...);
   ```

3. **EVMSubmitter (K.5)** reads finalized attestations from BaaLS and submits to RewardDistributor:
   - Polls `/api/v1/oracle/attestations/{chain_id}` every 30 seconds
   - Signs each attestation with the EVM bridge key
   - Submits to `RewardDistributor.submitDormancyProof()`
   - Tracks submission state: pending → confirmed / failed
   - Retries with exponential backoff (2^attempts minutes)

### Polygon Amoy (Spoke Chain, Optional)

For distributed dormancy verification, RewardDistributor can:
- Query Polygon Amoy RPC for secondary attestation
- Verify that attestation is also recorded on Amoy
- Increases security against single-chain oracle manipulation

Configure in config.toml:
```toml
[oracle]
amoy_rpc = "https://rpc-amoy.polygon.technology/"
```

## Monitoring & Alerts

### Check Oracle Status

```bash
# Health endpoint includes oracle info
curl -s http://localhost:18082/api/v1/health | jq '.oracle'

# Response:
{
  "oracle_enabled": true,
  "oracle_pubkey": "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc",
  "oracle_attestations_count": 150,
  "last_attestation_block": 1234567
}
```

### Monitor Logs

```bash
# Watch for attestation submissions
journalctl -u baalsd -f | grep "ORACLE\|EVM"

# Expected output:
[ORACLE] Attested dormancy: chain=bitcoin addr=1A1z7a... since_block=750000 at_baals_block=1234567
[EVM] Submitted attestation: chain=bitcoin addr=1A1z7a... tx=0xabcd1234...
[EVM] Submission confirmed (attempt 1): chain=bitcoin addr=1A1z7a...
```

### Alert on Failures

Via the monitor script (`/etc/baals/monitor.sh`):

```bash
BAALS_MONITOR_WEBHOOK_URL="https://hooks.slack.com/services/..." \
  /etc/baals/monitor.sh
```

Alerts on:
- Oracle node unreachable
- ChronoNode submission failures
- EVM submission failures (>3 attempts)
- Inconsistent attestation counts

## Security Considerations

1. **Signature Verification**: All attestations include ed25519 signatures from both ChronoNode and BaaLS. Verify both before using in EVM contracts.

2. **Key Management**:
   - Consensus key: Protected by Argon2id + AES-256-GCM encryption at rest.
   - EVM key: Separate secp256k1 key, also encrypted, can be rotated independently.
   - Use strong passwords (≥16 chars) and environment-locked key files.

3. **Replay Attack Prevention**:
   - Each attestation includes the BaaLS block index and hash at attestation time.
   - EVM contracts must verify that the block hash corresponds to a finalized BaaLS block.

4. **Oracle Address Authorization**:
   - EVM RewardDistributor must only accept attestations signed by the registered oracle address.
   - Use Timelock governance for oracle address updates to prevent frontrunning.

5. **Rate Limiting**:
   - ChronoNode should throttle attestation submissions (e.g., one per address per hour minimum).
   - BaaLS accepts submissions without rate limiting but stores in persistent storage for auditability.

## Testing

### Unit Tests

```bash
cargo test oracle_integration --all-features
```

Tests included:
- Dormancy proof signature verification
- Oracle attestation signing
- Storage key generation
- EVM submitter initialization
- Canonical message determinism

### Integration Tests (with Running Node)

```bash
# Start a local node
baalsd start --config ./local-node.toml &

# Run full integration suite
cargo test oracle -- --nocapture --test-threads=1

# Submit a test proof
curl -X POST http://localhost:18080/api/v1/oracle/attest \
  -H "Content-Type: application/json" \
  -d @test-proof.json

# Query attestations
curl http://localhost:18080/api/v1/oracle/attestations/bitcoin/1A1z7agoat7qcUeF
```

## Roadmap

- ⏳ K.1 Dedicated Oracle WASM contract (optional hardening; live path currently uses REST oracle namespace storage)
- ✅ K.2 REST: POST `/api/v1/oracle/attest`
- ✅ K.3 REST: GET `/api/v1/oracle/attestations/{chain_id}/{address}`
- ✅ K.4 EVM bridge keypair generation & loading
- ✅ K.5 EVMSubmitter module (poll, sign, submit, retry)
- ✅ K.6 Config structure with EVM settings
- ✅ K.7 Integration tests
- ✅ K.8 Operational key rotation runbook documented (`docs/RELAY_ORACLE_KEY_ROTATION.md`)
- 🔄 Enhanced: `admin rotate-evm-key` CLI command (atomic key rotation)
- 🔄 Enhanced: Proof anchoring to specific block heights
- 🔄 Enhanced: Multi-chain relay (Arbitrum → Polygon cross-chain verification)

## References

- **Resurgence Protocol**: [https://github.com/GrindSquad/resurgence-protocol](https://github.com/GrindSquad/resurgence-protocol)
- **ChronoNode**: [https://github.com/GrindSquad/chrononode](https://github.com/GrindSquad/chrononode)
- **BaaLS Documentation**: [./OPERATING.md](./OPERATING.md)
- **Rotation Runbook**: [./RELAY_ORACLE_KEY_ROTATION.md](./RELAY_ORACLE_KEY_ROTATION.md)
- **Dormancy Proof Format**: Defined in `src/oracle.rs::DormancyProof`
