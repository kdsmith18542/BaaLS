# Arweave Anchoring Notes (Phase M)

## Current State

- Manifest snapshot captured:
  - `deploy/baals-deployment-manifest-20260526T035006Z.json`
- Uploaded to Irys gateway:
  - `https://gateway.irys.xyz/8ifAT1ABc7p1gj4Zf6wsxAc1vfyQMJT96AVSAj2XrW5F`
- Anchor registry updated:
  - `deploy/arweave-anchors.json` (`status: uploaded`, non-null `tx_id`)

## Repeat / Roll Forward

Run:

```bash
scripts/anchor-deployment.sh \
  --api-url https://baals.network \
  --config /etc/baals/config.toml \
  --data-dir /var/lib/baals \
  --node-id 0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc
```

The script defaults to:
- `--irys-network devnet`
- `--irys-token ethereum`
- `--irys-provider-url` from `[oracle].evm_rpc`
- wallet from env var named in `[oracle].evm_private_key_env` (if exported)

If wallet is not exported in your shell, pass it explicitly:

```bash
scripts/anchor-deployment.sh ... --irys-wallet "$BAALS_EVM_PRIVATE_KEY"
```

## Verification

- Manifest file hash should match `manifest_sha256` in `deploy/arweave-anchors.json`.
- Gateway URL resolves:
  - `https://gateway.irys.xyz/<tx_id>`
