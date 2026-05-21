"""HTTP client for BaaLS node API."""

from __future__ import annotations

from dataclasses import dataclass

import httpx
from nacl.signing import SigningKey

from baals.auth import auth_token_request
from baals.transaction import Transaction


@dataclass
class Keypair:
    sk: SigningKey
    pk_hex: str

    def __init__(self, sk_hex: str):
        self.sk = SigningKey(bytes.fromhex(sk_hex))
        self.pk_hex = self.sk.verify_key.encode().hex()

    @staticmethod
    def generate() -> Keypair:
        sk = SigningKey.generate()
        kp = object.__new__(Keypair)
        kp.sk = sk
        kp.pk_hex = sk.verify_key.encode().hex()
        return kp

    @property
    def sk_hex(self) -> str:
        return bytes(self.sk).hex()


class BaaLSClient:
    """Client for a remote BaaLS node's HTTP API.

    Parameters
    ----------
    base_url : str
        Node URL, e.g. ``"https://198.71.49.148:18080"`` or ``"http://127.0.0.1:8080"``.
    node_keypair : Keypair | None
        Signing identity for JWT auth.  Required for write operations.
    verify_ssl : bool
        Pass ``False`` to skip TLS certificate verification (e.g. self-signed certs).
    """

    def __init__(
        self,
        base_url: str,
        node_keypair: Keypair | None = None,
        *,
        verify_ssl: bool = True,
        timeout: float = 30.0,
    ):
        self._base = base_url.rstrip("/")
        self._kp = node_keypair
        self._token: str | None = None
        self._http = httpx.Client(
            base_url=self._base,
            verify=verify_ssl,
            timeout=timeout,
        )

    def close(self):
        self._http.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    # ------------------------------------------------------------------
    # Auth
    # ------------------------------------------------------------------

    def authenticate(self, *, ttl_seconds: int = 900) -> str:
        if self._kp is None:
            raise ValueError("node_keypair required for authentication")
        body = auth_token_request(self._kp.sk, self._kp.pk_hex, ttl_seconds=ttl_seconds)
        resp = self._http.post("/api/v1/auth/token", json=body)
        resp.raise_for_status()
        self._token = resp.json()["token"]
        return self._token

    def _auth_headers(self) -> dict[str, str]:
        if self._token is None:
            self.authenticate()
        return {"Authorization": f"Bearer {self._token}"}

    # ------------------------------------------------------------------
    # Read endpoints
    # ------------------------------------------------------------------

    def health(self) -> dict:
        return self._http.get("/api/v1/health").json()

    def get_block_latest(self) -> dict:
        resp = self._http.get("/api/v1/blocks/latest")
        resp.raise_for_status()
        return resp.json()

    def get_block(self, height: int) -> dict:
        resp = self._http.get(f"/api/v1/blocks/{height}")
        resp.raise_for_status()
        return resp.json()

    def get_block_by_hash(self, hash_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/blocks/hash/{hash_hex}")
        resp.raise_for_status()
        return resp.json()

    def get_account(self, pubkey_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/accounts/{pubkey_hex}")
        resp.raise_for_status()
        return resp.json()

    def get_transaction(self, hash_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/transactions/{hash_hex}")
        resp.raise_for_status()
        return resp.json()

    def get_transaction_finality(self, hash_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/transactions/{hash_hex}", params={"finality": "true"})
        resp.raise_for_status()
        return resp.json()

    def get_transactions_by_address(self, pubkey_hex: str) -> list[dict]:
        resp = self._http.get(f"/api/v1/transactions/address/{pubkey_hex}")
        resp.raise_for_status()
        return resp.json()

    def get_supply(self) -> dict:
        resp = self._http.get("/api/v1/supply")
        resp.raise_for_status()
        return resp.json()

    def get_peers(self) -> list:
        resp = self._http.get("/api/v1/peers")
        resp.raise_for_status()
        return resp.json()

    def get_signers(self) -> list:
        resp = self._http.get("/api/v1/admin/signers")
        resp.raise_for_status()
        return resp.json()

    def get_account_proof(self, pubkey_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/proofs/account/{pubkey_hex}")
        resp.raise_for_status()
        return resp.json()

    def get_contract_proof(self, contract_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/proofs/contract/{contract_hex}")
        resp.raise_for_status()
        return resp.json()

    def get_contract_state(self, contract_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/contracts/{contract_hex}/state")
        resp.raise_for_status()
        return resp.json()

    def get_contract_storage(self, contract_hex: str, key_hex: str) -> dict:
        resp = self._http.get(f"/api/v1/contracts/{contract_hex}/storage/{key_hex}")
        resp.raise_for_status()
        return resp.json()

    # ------------------------------------------------------------------
    # Write endpoints
    # ------------------------------------------------------------------

    def submit_transaction(self, tx: Transaction) -> dict:
        resp = self._http.post(
            "/api/v1/transactions",
            json=tx.to_json(),
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def create_account(self, pubkey_hex: str, balance: int) -> dict:
        resp = self._http.post(
            "/api/v1/accounts",
            json={"pubkey": pubkey_hex, "balance": balance},
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def deploy_contract(self, tx: Transaction) -> dict:
        resp = self._http.post(
            "/api/v1/contracts/deploy",
            json=tx.to_json(),
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def invoke_contract(self, tx: Transaction) -> dict:
        resp = self._http.post(
            "/api/v1/contracts/invoke",
            json=tx.to_json(),
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def query_contract(self, contract_hex: str, method: str, payload: list[int] | None = None) -> dict:
        body: dict = {"contract_id": contract_hex, "method": method}
        if payload is not None:
            body["payload"] = payload
        resp = self._http.post("/api/v1/contracts/call", json=body)
        resp.raise_for_status()
        return resp.json()

    def estimate_gas(self, tx: Transaction) -> dict:
        resp = self._http.post(
            "/api/v1/contracts/estimate-gas",
            json=tx.to_json(),
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    # ------------------------------------------------------------------
    # Admin endpoints
    # ------------------------------------------------------------------

    def add_signer(self, pubkey_hex: str) -> dict:
        resp = self._http.post(
            "/api/v1/admin/signers",
            json={"public_key": pubkey_hex},
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def remove_signer(self, pubkey_hex: str) -> dict:
        resp = self._http.delete(
            f"/api/v1/admin/signers/{pubkey_hex}",
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def add_peer(self, address: str) -> dict:
        resp = self._http.post(
            "/api/v1/peers",
            json={"address": address},
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def remove_peer(self, address: str) -> dict:
        resp = self._http.delete(
            f"/api/v1/peers/{address}",
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    def trigger_sync(self) -> dict:
        resp = self._http.post(
            "/api/v1/sync/trigger",
            json={},
            headers=self._auth_headers(),
        )
        resp.raise_for_status()
        return resp.json()
