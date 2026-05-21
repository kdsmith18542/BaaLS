"""
Full fork resolution test:
1. Create accounts on both nodes
2. Cross-register signers
3. Submit tx on VPS (creates height 1 divergent block)
4. Submit 3 txs on local (creates height 1,2,3 — longer chain)
5. Report fork state
"""
import json, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey

LOCAL_URL = "http://127.0.0.1:8080"
LOCAL_NODE_SK = "b6dd606a5390798bcc4d1f81508064930bc76fdb0de139d6fd187f938d846092"
LOCAL_NODE_PK = "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"
VPS_NODE_PK = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc"
ALICE_SK = "aaaa000000000000000000000000000000000000000000000000000000000001"
ALICE_PK = SigningKey(bytes.fromhex(ALICE_SK)).verify_key.encode().hex()
BOB_PK = SigningKey(bytes.fromhex("bbbb000000000000000000000000000000000000000000000000000000000002")).verify_key.encode().hex()

def get_token(url, node_sk, node_pk):
    sk = SigningKey(bytes.fromhex(node_sk))
    ts = int(time.time())
    nonce = secrets.token_hex(16)
    sig = sk.sign(f"baals-auth-token:{ts}:{nonce}".encode()).signature.hex()
    body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": node_pk, "signature": sig, "ttl_seconds": 900}).encode()
    req = urllib.request.Request(f"{url}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())["token"]

def create_account(url, token, pubkey, balance):
    body = json.dumps({"pubkey": pubkey, "balance": balance}).encode()
    req = urllib.request.Request(f"{url}/api/v1/accounts", data=body, headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())

def add_signer(url, token, pk):
    body = json.dumps({"public_key": pk}).encode()
    req = urllib.request.Request(f"{url}/api/v1/admin/signers", data=body, headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())

def submit_tx(url, token, tx_nonce, amount):
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
    req = urllib.request.Request(f"{url}/api/v1/transactions", data=json.dumps(tx).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    return urllib.request.urlopen(req).read().decode()

def health(url):
    return json.loads(urllib.request.urlopen(f"{url}/api/v1/health").read().decode())

# Step 1: Create accounts on LOCAL
print("=== Step 1: Create accounts on LOCAL ===")
token = get_token(LOCAL_URL, LOCAL_NODE_SK, LOCAL_NODE_PK)
print(f"Alice: {create_account(LOCAL_URL, token, ALICE_PK, 1000000)}")
print(f"Bob:   {create_account(LOCAL_URL, token, BOB_PK, 0)}")

# Step 2: Add VPS signer on LOCAL
print("\n=== Step 2: Cross-register signers on LOCAL ===")
print(f"Add VPS signer: {add_signer(LOCAL_URL, token, VPS_NODE_PK)}")

print("\n=== Step 3: Submit 3 txs on LOCAL (amounts 10, 20, 30) ===")
for nonce, amt in [(1, 10), (2, 20), (3, 30)]:
    try:
        result = submit_tx(LOCAL_URL, token, nonce, amt)
        print(f"  TX nonce={nonce} amt={amt}: {result}")
    except urllib.error.HTTPError as e:
        print(f"  TX nonce={nonce} Error: {e.read().decode()}")
    time.sleep(6)

h = health(LOCAL_URL)
print(f"\nLOCAL: height={h['latest_block_index']}, hash={h['latest_block_hash'][:16]}...")
