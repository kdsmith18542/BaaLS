import json, urllib.request, time, secrets
from nacl.signing import SigningKey

sk_hex = "afd1fb8ddccc63ca29b2f11f371d9b959e1b07f217788ed04e2d1d18b50e7efd"
pk_hex = "0a82b7b0d6be0cde841d31fda2a0c9ceff7636c81332bc2ed9cc981f5f537abc"
url = "http://127.0.0.1:18080"

sk = SigningKey(bytes.fromhex(sk_hex))
ts = int(time.time())
nonce = secrets.token_hex(16)
challenge = f"baals-auth-token:{ts}:{nonce}"
sig = sk.sign(challenge.encode()).signature.hex()

body = json.dumps({"timestamp": ts, "nonce": nonce, "public_key": pk_hex, "signature": sig, "ttl_seconds": 900}).encode()
req = urllib.request.Request(f"{url}/api/v1/auth/token", data=body, headers={"Content-Type": "application/json"}, method="POST")
token = json.loads(urllib.request.urlopen(req).read().decode())["token"]

for (pk, bal) in [("aaaa000000000000000000000000000000000000000000000000000000000001", 1000000), ("bbbb000000000000000000000000000000000000000000000000000000000002", 0)]:
    body = json.dumps({"pubkey": pk, "balance": bal}).encode()
    req = urllib.request.Request(f"{url}/api/v1/accounts", data=body, headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"}, method="POST")
    resp = urllib.request.urlopen(req).read().decode()
    print(f"Account {pk[:8]}...: {resp}")
