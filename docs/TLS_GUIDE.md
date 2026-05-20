# TLS Guide for BaaLS

## Overview

BaaLS supports TLS encryption for P2P node-to-node communication and HTTPS API serving. When enabled, all sync traffic between peers is encrypted and authenticated using mutual TLS (mTLS).

## Quick Start

### Option A: Use the built-in CLI command

```bash
# Generate a self-signed certificate and key
baalsd admin tls-generate --output ./certs --cn "node1.baals.local"
```

This produces `./certs/server.crt` and `./certs/server.key` ready for use.

### Option B: Manual generation with OpenSSL

```bash
# Generate CA key and certificate
openssl req -x509 -newkey rsa:4096 -keyout ca-key.pem -out ca-cert.pem \
    -days 3650 -nodes -subj "/CN=BaaLS CA"

# Generate node key and CSR
openssl genrsa -out node-key.pem 4096
openssl req -new -key node-key.pem -out node.csr \
    -subj "/CN=node1.baals.local"

# Sign with CA
openssl x509 -req -in node.csr -CA ca-cert.pem -CAkey ca-key.pem \
    -CAcreateserial -out node-cert.pem -days 365
```

### Configure BaaLS

Edit `config.toml`:
```toml
[network]
tls_enabled = true
tls_cert_path = "./certs/node-cert.pem"
tls_key_path = "./certs/node-key.pem"
tls_ca_cert_path = "./certs/ca-cert.pem"
```

### Start node
```bash
baalsd node start --data-dir ./data --peer <peer_addr>:9070
```

## Certificate Pinning

For additional security, BaaLS supports certificate pinning. The node's certificate fingerprint is exchanged during the P2P handshake.

## mTLS Configuration

When `tls_ca_cert_path` is set, BaaLS requires all connecting peers to present a certificate signed by the same CA (mutual TLS). This ensures only authorized nodes can join the network.

## Production Deployment

### Certificate Rotation
1. Generate new certificate signed by existing CA
2. Update `tls_cert_path` and `tls_key_path` in config
3. Restart node
4. Old connections drain naturally

### CA Compromise
If the CA key is compromised:
1. Generate a new CA
2. Re-issue all node certificates
3. Update `tls_ca_cert_path` on all nodes
4. Restart all nodes

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `TLS error: certificate expired` | Certificate expired | Renew certificate |
| `TLS error: unknown CA` | CA certificate mismatch | Ensure all nodes use same CA |
| Connection refused | TLS handshake failure | Check cert/key paths in config |
| Peer rejected after handshake | Certificate CN mismatch | Verify certificate subject matches node ID |

## Security Considerations

- Keep CA private key offline when not in use
- Use unique certificates per node, not shared
- Set appropriate validity periods (90 days recommended for production)
- Monitor certificate expiry and rotate proactively
- Use hardware security module (HSM) for CA key storage in production
