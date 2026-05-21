#!/usr/bin/env python3
"""Get a JWT admin token from a BaaLS node. Usage: get_token.py <sk_hex> <pk_hex> <url>"""
import json, sys, time, secrets, urllib.request

try:
    from nacl.signing import SigningKey
except ImportError:
    print("pip install pynacl", file=sys.stderr)
    sys.exit(1)

sk_hex = sys.argv[1]
pk_hex = sys.argv[2]
url = sys.argv[3]

sk = SigningKey(bytes.fromhex(sk_hex))
timestamp = int(time.time())
nonce = secrets.token_hex(16)
challenge = f"baals-auth-token:{timestamp}:{nonce}"
signature = sk.sign(challenge.encode()).signature.hex()

body = json.dumps({
    "timestamp": timestamp,
    "nonce": nonce,
    "public_key": pk_hex,
    "signature": signature,
    "ttl_seconds": 900,
}).encode()

req = urllib.request.Request(
    f"{url}/api/v1/auth/token",
    data=body,
    headers={"Content-Type": "application/json"},
    method="POST",
)
resp = json.loads(urllib.request.urlopen(req).read().decode())
print(resp["token"])
