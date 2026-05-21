"""Submit 3 txs on local to create divergent blocks 13, 14, 15. Local chain will be longer."""
import json, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey

LOCAL_URL = "http://127.0.0.1:8080"
LOCAL_NODE_SK = "b6dd606a5390798bcc4d1f81508064930bc76fdb0de139d6fd187f938d846092"
LOCAL_NODE_PK = "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"
ALICE_SK = "aaaa000000000000000000000000000000000000000000000000000000000001"
ALICE_PK = SigningKey(bytes.fromhex(ALICE_SK)).verify_key.encode().hex()
BOB_PK = SigningKey(bytes.fromhex("bbbb000000000000000000000000000000000000000000000000000000000002")).verify_key.encode().hex()

def get_token():
    sk = SigningKey(bytes.fromhex(LOCAL_NODE_SK))
    ts = int(time.time())
    nonce = secrets.token_hex(16)
    sig = sk.sign(f"baals-auth-token:{ts}:{nonce}".encode()).signature.hex()
    body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": LOCAL_NODE_PK, "signature": sig, "ttl_seconds": 900}).encode()
    req = urllib.request.Request(f"{LOCAL_URL}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
    return json.loads(urllib.request.urlopen(req).read().decode())["token"]

def submit_tx(token, tx_nonce, amount):
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
    req = urllib.request.Request(f"{LOCAL_URL}/api/v1/transactions", data=json.dumps(tx).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    return urllib.request.urlopen(req).read().decode()

token = get_token()

# Submit 3 txs with different amounts (unique to local fork)
for i, (nonce, amt) in enumerate([(13, 10), (14, 20), (15, 30)], 1):
    try:
        result = submit_tx(token, nonce, amt)
        print(f"TX{i} (nonce={nonce}, amt={amt}): {result}")
    except urllib.error.HTTPError as e:
        print(f"TX{i} Error {e.code}: {e.read().decode()}")
    # Wait for block production between txs
    time.sleep(6)

health = json.loads(urllib.request.urlopen(f"{LOCAL_URL}/api/v1/health").read().decode())
print(f"\nLocal: height={health['latest_block_index']}, hash={health['latest_block_hash'][:16]}...")
