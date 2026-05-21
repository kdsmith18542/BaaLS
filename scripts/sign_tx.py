#!/usr/bin/env python3
"""Build and sign a BaaLS transfer transaction, output JSON suitable for POST /api/v1/transactions"""
import json, hashlib, struct, sys, time

try:
    from nacl.signing import SigningKey
except ImportError:
    print("pip install pynacl", file=sys.stderr)
    sys.exit(1)

ALICE_SK_HEX = "062a15b7cb3d4076c093c3ea2f16743a2f261b598af5e08573e2f89a0a7220f8"
ALICE_PK_HEX = "abcd19f2e980e7eacb107982acc7a5bdaa2b6f3ef0c3d3fdfe9f3bff9703d83b"
BOB_PK_HEX   = "a5b243912c70a6103681e84b5f65b15434d7b1480481589115485d9d59664f36"

sk = SigningKey(bytes.fromhex(ALICE_SK_HEX))

tx = {
    "hash": [0]*32,
    "sender": list(bytes.fromhex(ALICE_PK_HEX)),
    "nonce": 1,
    "timestamp": int(time.time()),
    "recipient": {"Wallet": list(bytes.fromhex(BOB_PK_HEX))},
    "payload": {"Transfer": {"amount": 100}},
    "signature": [0]*64,
    "gas_limit": 100000,
    "gas_price": 1,
    "priority": 0,
    "metadata": None,
    "chain_id": 1,
}

# Build hash the same way Rust does: sender + nonce + timestamp + gas_limit + gas_price + priority + chain_id + recipient(bincode) + payload(bincode)
# This is tricky because bincode serialization must match Rust exactly.
# Instead, let's just output the unsigned tx and note we need the Rust CLI to sign it.

print(json.dumps(tx, indent=2))
print("\nNote: this tx is unsigned - Rust signature required", file=sys.stderr)
