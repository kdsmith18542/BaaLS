import json, time, secrets, urllib.request
from nacl.signing import SigningKey

VPS_URL = "http://127.0.0.1:18080"
VPS_NODE_SK = "afd1fb8ddccc63ca29b2f11f371d9b959e1b07f217788ed04e2d1d18b50e7efd"
VPS_NODE_PK = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc"
ALICE_PK = SigningKey(bytes.fromhex("aaaa000000000000000000000000000000000000000000000000000000000001")).verify_key.encode().hex()
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

# Add local signer
data = json.dumps({"public_key": "0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0"}).encode()
req = urllib.request.Request(f"{VPS_URL}/api/v1/admin/signers", data=data, headers={"Content-Type": "application/json"}, method="POST")
print("Signer:", urllib.request.urlopen(req).read().decode())

# Create accounts
for pk, bal in [(ALICE_PK, 1000000), (BOB_PK, 0)]:
    data = json.dumps({"pubkey": pk, "balance": bal}).encode()
    req = urllib.request.Request(f"{VPS_URL}/api/v1/accounts", data=data, headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    print(f"Account {pk[:12]}...: {urllib.request.urlopen(req).read().decode()}")
