"""JWT auth token generation for BaaLS admin API."""

from __future__ import annotations

import secrets
import time

from nacl.signing import SigningKey


def auth_challenge(timestamp: int | None = None, nonce: str | None = None) -> tuple[str, int, str]:
    ts = timestamp if timestamp is not None else int(time.time())
    n = nonce if nonce is not None else secrets.token_hex(16)
    return f"baals-auth-token:{ts}:{n}", ts, n


def auth_token_request(
    sk: SigningKey | bytes | str,
    pk_hex: str,
    *,
    ttl_seconds: int = 900,
) -> dict:
    if isinstance(sk, str):
        sk = SigningKey(bytes.fromhex(sk))
    elif isinstance(sk, bytes):
        sk = SigningKey(sk)
    challenge, ts, nonce = auth_challenge()
    sig = sk.sign(challenge.encode()).signature.hex()
    return {
        "timestamp": ts,
        "nonce": nonce,
        "public_key": pk_hex,
        "signature": sig,
        "ttl_seconds": ttl_seconds,
    }
