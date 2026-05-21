import json, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey

LOCAL_URL = "http://127.0.0.1:8080"
LOCAL_NODE_SK = "b6dd606a5390798bcc4d1f81508064930bc76fdb0de139d6fd187f938d846092"
LOCAL_NODE_PK = "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"
ALICE_SK = "aaaa000000000000000000000000000000000000000000000000000000000001"
ALICE_PK = SigningKey(bytes.fromhex(ALICE_SK)).verify_key.encode().hex()
BOB_PK = SigningKey(bytes.fromhex("bbbb000000000000000000000000000000000000000000000000000000000002")).verify_key.encode().hex()

# Get token
sk = SigningKey(bytes.fromhex(LOCAL_NODE_SK))
ts = int(time.time())
nonce = secrets.token_hex(16)
challenge = f"baals-auth-token:{ts}:{nonce}"
sig = sk.sign(challenge.encode()).signature.hex()
body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": LOCAL_NODE_PK, "signature": sig, "ttl_seconds": 900}).encode()
req = urllib.request.Request(f"{LOCAL_URL}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
token = json.loads(urllib.request.urlopen(req).read().decode())["token"]

# Build tx
tx_ts = int(time.time())
buf = bytes.fromhex(ALICE_PK)
buf += struct.pack("<Q", 1)  # nonce (starts at 1)
buf += struct.pack("<Q", tx_ts)
buf += struct.pack("<Q", 100000)  # gas_limit
buf += struct.pack("<Q", 1)  # gas_price
buf += struct.pack("<B", 0)  # priority
buf += struct.pack("<Q", 1)  # chain_id
# bincode: Address::Wallet = variant_index(u32) + slice_len(u64) + bytes
buf += struct.pack("<I", 0) + struct.pack("<Q", 32) + bytes.fromhex(BOB_PK)
# bincode: TransactionPayload::Transfer{amount} = variant_index(u32) + amount(u64)
buf += struct.pack("<I", 0) + struct.pack("<Q", 100)

tx_hash = hashlib.sha256(buf).digest()
sender_sk = SigningKey(bytes.fromhex(ALICE_SK))
tx_sig = sender_sk.sign(tx_hash).signature

tx = {
    "hash": list(tx_hash),
    "sender": list(bytes.fromhex(ALICE_PK)),
    "nonce": 1,
    "timestamp": tx_ts,
    "recipient": {"Wallet": list(bytes.fromhex(BOB_PK))},
    "payload": {"Transfer": {"amount": 100}},
    "signature": list(tx_sig),
    "gas_limit": 100000,
    "gas_price": 1,
    "priority": 0,
    "metadata": None,
    "chain_id": 1,
}

try:
    req = urllib.request.Request(
        f"{LOCAL_URL}/api/v1/transactions",
        data=json.dumps(tx).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        method="POST",
    )
    print(urllib.request.urlopen(req).read().decode())
except urllib.error.HTTPError as e:
    print(f"Error {e.code}: {e.read().decode()}")
