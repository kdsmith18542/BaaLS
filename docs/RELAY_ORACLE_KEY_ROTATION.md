# Relay and Oracle EVM Key Rotation Runbook

This runbook closes plan items `K.8` and `L.6`.

## Scope

- Rotates the secp256k1 key used by both:
  - `EVMSubmitter` (Phase K oracle submissions)
  - `RelayWatcher` (Phase L bridge relay)
- This key is loaded from `oracle.evm_private_key_env` and currently maps to `0x201624cBa366250D08bCdA95e6eF64151687A447`.

Not covered here:
- Ed25519 consensus key rotation (`baalsd admin rotate-consensus-key`)

## Pre-Flight Checklist

1. Generate a new secp256k1 private key offline and derive the new EVM address.
2. Prepare governance actions on RewardDistributor to grant both roles to the new address:
   - `DORMANCY_ORACLE_ROLE`
   - `RELAY_MINTER_ROLE`
3. Confirm BaaLS config contains the correct hub contract and RPC:
   - `[oracle].reward_distributor`
   - `[oracle].evm_rpc`
4. Schedule a maintenance window for restart/reload.

## Rotation Procedure

1. Add the new private key as a new environment variable on each node host:

```bash
# Example only; use your actual secret management method.
export BAALS_EVM_PRIVATE_KEY_NEXT="0x<new_32_byte_hex>"
```

2. Grant roles to the new address on RewardDistributor via governance:
   - `grantRole(DORMANCY_ORACLE_ROLE, <new_address>)`
   - `grantRole(RELAY_MINTER_ROLE, <new_address>)`
   - Wait for timelock and execute.

3. Update BaaLS config to point at the new env var:

```toml
[oracle]
evm_private_key_env = "BAALS_EVM_PRIVATE_KEY_NEXT"
```

4. Restart node services:

```bash
sudo systemctl restart baalsd
sudo systemctl restart baalsd-node2
```

5. Verify both background workers picked up the new signer:

```bash
journalctl -u baalsd -n 200 --no-pager | grep -E "\[EVM\] Submitter initialized|\[Relay\] Watcher initialized"
journalctl -u baalsd-node2 -n 200 --no-pager | grep -E "\[EVM\] Submitter initialized|\[Relay\] Watcher initialized"
```

Expected log lines include:
- `[EVM] Submitter initialized. Signing address: <new_address>`
- `[Relay] Watcher initialized. Signing address: <new_address>`

6. Run canary transactions:
   - Submit one controlled oracle attestation and confirm hub tx success.
   - Trigger one controlled `bridgeClaimRelay()` on a spoke and confirm `mintForRelay` success on hub.

7. After canary success, revoke old roles from the previous address (governance):
   - `revokeRole(DORMANCY_ORACLE_ROLE, <old_address>)`
   - `revokeRole(RELAY_MINTER_ROLE, <old_address>)`

## Rollback

1. Repoint `oracle.evm_private_key_env` back to the old variable.
2. Restart BaaLS services.
3. If old roles were already revoked, re-grant them via governance.
4. Re-run canary checks.

## Evidence to Record

- Governance proposal IDs + execution tx hashes for grants/revokes.
- Journal excerpts showing the new signer address for both workers.
- First successful oracle tx hash signed by the new key.
- First successful relay `mintForRelay` tx hash signed by the new key.
