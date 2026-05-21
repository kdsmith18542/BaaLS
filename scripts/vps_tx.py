"""Submit a transfer tx on VPS to produce a divergent block."""
import json, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey

VPS_URL = "http://127.0.0.1:18080"
VPS_NODE_SK = "afd1fb8ddccc63ca29b2f11f371d9b959e1b07f217788ed04e2d1d18b50e7efd"
VPS_NODE_PK = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc"
ALICE_SK = "aaaa000000000000000000000000000000000000000000000000000000000001"
ALICE_PK = SigningKey(bytes.fromhex(ALICE_SK)).verify_key.encode().hex()
BOB_PK = SigningKey(bytes.fromhex("bbbb000000000000000000000000000000000000000000000000000000000002")).verify_key.encode().hex()

# Get token
sk = SigningKey(bytes.fromhex(VPS_NODE_SK))
ts = int(time.time())
nonce = secrets.token_hex(16)
challenge = f"baals-auth-token:{ts}:{nonce}"
sig = sk.sign(challenge.encode()).signature.hex()
body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": VPS_NODE_PK, "signature": sig, "ttl_seconds": 900}).encode()
req = urllib.request.Request(f"{VPS_URL}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
token = json.loads(urllib.request.urlopen(req).read().decode())["token"]

# Build tx with DIFFERENT amount to guarantee different hash
tx_ts = int(time.time())
amount = 50  # Different amount than local (which used 100)
tx_nonce = 1

buf = bytes.fromhex(ALICE_PK)
buf += struct.pack("<Q", tx_nonce)
buf += struct.pack("<Q", tx_ts)
buf += struct.pack("<Q", 100000)
buf += struct.pack("<Q", 1)
buf += struct.pack("<B", 0)
buf += struct.pack("<Q", 1)
buf += struct.pack("<I", 0) + struct.pack("<Q", 32) + bytes.fromhex(BOB_PK)
buf += struct.pack("<I", 0) + struct.pack("<Q", amount)

tx_hash = hashlib.sha256(buf).digest()
sender_sk = SigningKey(bytes.fromhex(ALICE_SK))
tx_sig = sender_sk.sign(tx_hash).signature

tx = {
    "hash": list(tx_hash),
    "sender": list(bytes.fromhex(ALICE_PK)),
    "nonce": tx_nonce,
    "timestamp": tx_ts,
    "recipient": {"Wallet": list(bytes.fromhex(BOB_PK))},
    "payload": {"Transfer": {"amount": amount}},
    "signature": list(tx_sig),
    "gas_limit": 100000,
    "gas_price": 1,
    "priority": 0,
    "metadata": None,
    "chain_id": 1,
}

try:
    req = urllib.request.Request(
        f"{VPS_URL}/api/v1/transactions",
        data=json.dumps(tx).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        method="POST",
    )
    resp = urllib.request.urlopen(req).read().decode()
    print(f"TX submitted: {resp}")
except urllib.error.HTTPError as e:
    print(f"Error {e.code}: {e.read().decode()}")

# Check health
import time as t
t.sleep(7)
req = urllib.request.Request(f"{VPS_URL}/api/v1/health")
health = json.loads(urllib.request.urlopen(req).read().decode())
print(f"VPS height: {health['latest_block_index']}, hash: {health['latest_block_hash']}")
