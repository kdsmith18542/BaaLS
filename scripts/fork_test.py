#!/usr/bin/env python3
"""Fork resolution test: both nodes produce independently, then sync and resolve."""
import json, time, secrets, struct, hashlib, urllib.request, sys
from nacl.signing import SigningKey

# --- Config ---
LOCAL_URL = "http://127.0.0.1:8080"
VPS_URL = "http://127.0.0.1:18080"  # accessed via SSH port-forward or directly

LOCAL_NODE_SK = "b6dd606a5390798bcc4d1f81508064930bc76fdb0de139d6fd187f938d846092"
LOCAL_NODE_PK = "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"

ALICE_SK = "aaaa000000000000000000000000000000000000000000000000000000000001"
ALICE_PK = SigningKey(bytes.fromhex(ALICE_SK)).verify_key.encode().hex()
BOB_SK = "bbbb000000000000000000000000000000000000000000000000000000000002"
BOB_PK = SigningKey(bytes.fromhex(BOB_SK)).verify_key.encode().hex()


def get_token(url, sk_hex, pk_hex):
    sk = SigningKey(bytes.fromhex(sk_hex))
    ts = int(time.time())
    nonce = secrets.token_hex(16)
    challenge = f"baals-auth-token:{ts}:{nonce}"
    sig = sk.sign(challenge.encode()).signature.hex()
    body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": pk_hex, "signature": sig, "ttl_seconds": 900}).encode()
    req = urllib.request.Request(f"{url}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())["token"]


def api_get(url, path):
    req = urllib.request.Request(f"{url}{path}", headers={"Accept": "application/json"})
    return json.loads(urllib.request.urlopen(req).read().decode())


def api_post(url, path, data, token=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    body = json.dumps(data).encode()
    req = urllib.request.Request(f"{url}{path}", data=body, headers=headers, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())


def build_transfer_tx(sender_sk_hex, sender_pk_hex, recipient_pk_hex, amount, nonce, chain_id=1):
    tx_ts = int(time.time())
    gas_limit = 100000
    gas_price = 1
    priority = 0

    buf = bytes.fromhex(sender_pk_hex)
    buf += struct.pack("<Q", nonce)
    buf += struct.pack("<Q", tx_ts)
    buf += struct.pack("<Q", gas_limit)
    buf += struct.pack("<Q", gas_price)
    buf += struct.pack("<B", priority)
    buf += struct.pack("<Q", chain_id)
    buf += struct.pack("<I", 0) + bytes.fromhex(recipient_pk_hex)  # Wallet variant
    buf += struct.pack("<I", 0) + struct.pack("<Q", amount)  # Transfer variant

    tx_hash = hashlib.sha256(buf).digest()
    sender_sk = SigningKey(bytes.fromhex(sender_sk_hex))
    tx_sig = sender_sk.sign(tx_hash).signature

    return {
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


def main():
    url = LOCAL_URL
    node_sk = LOCAL_NODE_SK
    node_pk = LOCAL_NODE_PK

    print(f"\n=== Phase 1: Setup accounts on LOCAL ({url}) ===")
    token = get_token(url, node_sk, node_pk)
    print(f"  Token: {token[:30]}...")

    # Register signers
    try:
        resp = api_post(url, "/api/v1/admin/signers", {"public_key": "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc"})
        print(f"  Register VPS signer: {resp}")
    except Exception as e:
        print(f"  VPS signer: {e}")

    # Create accounts
    api_post(url, "/api/v1/accounts", {"pubkey": ALICE_PK, "balance": 1000000}, token)
    api_post(url, "/api/v1/accounts", {"pubkey": BOB_PK, "balance": 0}, token)
    print(f"  Accounts created: alice={ALICE_PK[:16]}... bob={BOB_PK[:16]}...")

    print(f"\n=== Phase 2: Submit tx on LOCAL to produce a block ===")
    tx1 = build_transfer_tx(ALICE_SK, ALICE_PK, BOB_PK, 100, 0)
    resp = api_post(url, "/api/v1/transactions", tx1, token)
    print(f"  TX submitted: {resp}")

    # Wait for block production
    print("  Waiting for block production...")
    time.sleep(8)

    health = api_get(url, "/api/v1/health")
    print(f"  Local height: {health['latest_block_index']}")
    print(f"  Local hash: {health['latest_block_hash']}")

    # Submit a second tx
    tx2 = build_transfer_tx(ALICE_SK, ALICE_PK, BOB_PK, 200, 1)
    resp = api_post(url, "/api/v1/transactions", tx2, token)
    print(f"  TX2 submitted: {resp}")
    time.sleep(8)

    health = api_get(url, "/api/v1/health")
    print(f"  Local height: {health['latest_block_index']}")
    print(f"  Local hash: {health['latest_block_hash']}")

    print(f"\n=== DONE - Local has {health['latest_block_index']} blocks ===")
    print(f"Now run this on VPS to create divergent blocks, then connect.")


if __name__ == "__main__":
    main()
