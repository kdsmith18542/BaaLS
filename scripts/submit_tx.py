#!/usr/bin/env python3
"""Submit a signed transfer transaction to a BaaLS node.
Usage: submit_tx.py <node_sk_hex> <node_pk_hex> <node_url> <sender_sk_hex> <sender_pk_hex> <recipient_pk_hex> <amount> <nonce>"""
import json, sys, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey

node_sk_hex = sys.argv[1]
node_pk_hex = sys.argv[2]
node_url = sys.argv[3]
sender_sk_hex = sys.argv[4]
sender_pk_hex = sys.argv[5]
recipient_pk_hex = sys.argv[6]
amount = int(sys.argv[7])
nonce = int(sys.argv[8])

# Get auth token
node_sk = SigningKey(bytes.fromhex(node_sk_hex))
ts = int(time.time())
auth_nonce = secrets.token_hex(16)
challenge = f"baals-auth-token:{ts}:{auth_nonce}"
auth_sig = node_sk.sign(challenge.encode()).signature.hex()
auth_body = json.dumps({"timestamp": ts, "nonce": auth_nonce, "public_key": node_pk_hex, "signature": auth_sig, "ttl_seconds": 900}).encode()
req = urllib.request.Request(f"{node_url}/api/v1/auth/token", data=auth_body, headers={"Content-Type": "application/json"}, method="POST")
token = json.loads(urllib.request.urlopen(req).read().decode())["token"]

# Build transaction hash matching Rust: sender(32) + nonce(u64 LE) + timestamp(u64 LE) + gas_limit(u64 LE) + gas_price(u64 LE) + priority(u8) + chain_id(u64 LE) + recipient_tag(u32 LE=0 for Wallet) + recipient(32) + payload_tag(u32 LE=0 for Transfer) + amount(u64 LE)
tx_ts = int(time.time())
gas_limit = 100000
gas_price = 1
priority = 0
chain_id = 1

buf = b""
buf += bytes.fromhex(sender_pk_hex)
buf += struct.pack("<Q", nonce)
buf += struct.pack("<Q", tx_ts)
buf += struct.pack("<Q", gas_limit)
buf += struct.pack("<Q", gas_price)
buf += struct.pack("<B", priority)
buf += struct.pack("<Q", chain_id)
# Recipient: Wallet variant = tag 0 + 32 bytes pubkey
buf += struct.pack("<I", 0)
buf += bytes.fromhex(recipient_pk_hex)
# Payload: Transfer variant = tag 0 + amount u64
buf += struct.pack("<I", 0)
buf += struct.pack("<Q", amount)

tx_hash = hashlib.sha256(buf).digest()

# Sign the tx hash
sender_sk = SigningKey(bytes.fromhex(sender_sk_hex))
tx_sig = sender_sk.sign(tx_hash).signature

tx = {
    "hash": list(tx_hash),
    "sender": list(bytes.fromhex(sender_pk_hex)),
    "nonce": nonce,
    "timestamp": tx_ts,
    "recipient": {"Wallet": list(bytes.fromhex(recipient_pk_hex))},
    "payload": {"Transfer": {"amount": amount}},
    "signature": list(tx_sig),
    "gas_limit": gas_limit,
    "gas_price": gas_price,
    "priority": priority,
    "metadata": None,
    "chain_id": chain_id,
}

body = json.dumps(tx).encode()
req = urllib.request.Request(
    f"{node_url}/api/v1/transactions",
    data=body,
    headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
    method="POST",
)
try:
    resp = urllib.request.urlopen(req).read().decode()
    print(resp)
except urllib.error.HTTPError as e:
    print(f"HTTP {e.code}: {e.read().decode()}", file=sys.stderr)
    sys.exit(1)
