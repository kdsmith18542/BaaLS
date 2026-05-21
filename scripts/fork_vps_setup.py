"""Setup VPS side of fork: create accounts, add local signer, submit divergent tx."""
import json, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey

VPS_URL = "http://127.0.0.1:18080"
VPS_NODE_SK = "afd1fb8ddccc63ca29b2f11f371d9b959e1b07f217788ed04e2d1d18b50e7efd"
VPS_NODE_PK = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc"
LOCAL_NODE_PK = "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"
ALICE_SK = "aaaa000000000000000000000000000000000000000000000000000000000001"
ALICE_PK = SigningKey(bytes.fromhex(ALICE_SK)).verify_key.encode().hex()
BOB_PK = SigningKey(bytes.fromhex("bbbb000000000000000000000000000000000000000000000000000000000002")).verify_key.encode().hex()

def get_token():
    sk = SigningKey(bytes.fromhex(VPS_NODE_SK))
    ts = int(time.time())
    nonce = secrets.token_hex(16)
    sig = sk.sign(f"baals-auth-token:{ts}:{nonce}".encode()).signature.hex()
    body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": VPS_NODE_PK, "signature": sig, "ttl_seconds": 900}).encode()
    req = urllib.request.Request(f"{VPS_URL}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())["token"]

token = get_token()

# Create accounts
for pk, name, bal in [(ALICE_PK, "Alice", 1000000), (BOB_PK, "Bob", 0)]:
    body = json.dumps({"pubkey": pk, "balance": bal}).encode()
    req = urllib.request.Request(f"{VPS_URL}/api/v1/accounts", data=body, headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    print(f"{name}: {urllib.request.urlopen(req).read().decode()}")

# Add local signer
body = json.dumps({"public_key": LOCAL_NODE_PK}).encode()
req = urllib.request.Request(f"{VPS_URL}/api/v1/admin/signers", data=body, headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
print(f"Add local signer: {urllib.request.urlopen(req).read().decode()}")

# Submit divergent tx: nonce=1, amount=999 (different from local's 10)
tx_nonce = 1
amount = 999
tx_ts = int(time.time())
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
tx_sig = SigningKey(bytes.fromhex(ALICE_SK)).sign(tx_hash).signature
tx = {
    "hash": list(tx_hash), "sender": list(bytes.fromhex(ALICE_PK)),
    "nonce": tx_nonce, "timestamp": tx_ts,
    "recipient": {"Wallet": list(bytes.fromhex(BOB_PK))},
    "payload": {"Transfer": {"amount": amount}},
    "signature": list(tx_sig), "gas_limit": 100000, "gas_price": 1,
    "priority": 0, "metadata": None, "chain_id": 1,
}
try:
    req = urllib.request.Request(f"{VPS_URL}/api/v1/transactions", data=json.dumps(tx).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    print(f"VPS TX: {urllib.request.urlopen(req).read().decode()}")
except urllib.error.HTTPError as e:
    print(f"TX Error: {e.read().decode()}")

time.sleep(7)
health = json.loads(urllib.request.urlopen(f"{VPS_URL}/api/v1/health").read().decode())
print(f"\nVPS: height={health['latest_block_index']}, hash={health['latest_block_hash'][:16]}...")
