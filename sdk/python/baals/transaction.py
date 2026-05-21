"""Build, hash, and sign BaaLS transactions.

The hash algorithm matches ``Transaction::calculate_hash`` in ``src/types.rs``:
    SHA-256 over sender(raw 32) | nonce(u64 LE) | timestamp(u64 LE) |
    gas_limit(u64 LE) | gas_price(u64 LE) | priority(u8) | chain_id(u64 LE) |
    bincode(recipient) | bincode(payload) [| bincode(metadata)]
"""

from __future__ import annotations

import hashlib
import struct
import time

from nacl.signing import SigningKey

from baals.types import AddressType, PayloadType, _btreemap_str_str


class Transaction:
    def __init__(
        self,
        sender_pk: bytes,
        recipient: AddressType,
        payload: PayloadType,
        nonce: int,
        *,
        gas_limit: int = 100_000,
        gas_price: int = 1,
        priority: int = 0,
        chain_id: int = 1,
        timestamp: int | None = None,
        metadata: dict[str, str] | None = None,
    ):
        self.sender_pk = sender_pk if isinstance(sender_pk, bytes) else bytes.fromhex(sender_pk)
        self.recipient = recipient
        self.payload = payload
        self.nonce = nonce
        self.gas_limit = gas_limit
        self.gas_price = gas_price
        self.priority = priority
        self.chain_id = chain_id
        self.timestamp = timestamp if timestamp is not None else int(time.time())
        self.metadata = metadata
        self._hash: bytes | None = None
        self._signature: bytes | None = None

    def calculate_hash(self) -> bytes:
        buf = self.sender_pk
        buf += struct.pack("<Q", self.nonce)
        buf += struct.pack("<Q", self.timestamp)
        buf += struct.pack("<Q", self.gas_limit)
        buf += struct.pack("<Q", self.gas_price)
        buf += struct.pack("<B", self.priority)
        buf += struct.pack("<Q", self.chain_id)
        buf += self.recipient.bincode()
        buf += self.payload.bincode()
        if self.metadata is not None:
            buf += _btreemap_str_str(self.metadata)
        return hashlib.sha256(buf).digest()

    def sign(self, sk: SigningKey | bytes | str) -> Transaction:
        if isinstance(sk, str):
            sk = SigningKey(bytes.fromhex(sk))
        elif isinstance(sk, bytes):
            sk = SigningKey(sk)
        self._hash = self.calculate_hash()
        self._signature = sk.sign(self._hash).signature
        return self

    @property
    def hash(self) -> bytes:
        if self._hash is None:
            self._hash = self.calculate_hash()
        return self._hash

    @property
    def signature(self) -> bytes:
        if self._signature is None:
            raise ValueError("Transaction not signed; call .sign(sk) first")
        return self._signature

    def to_json(self) -> dict:
        return {
            "hash": list(self.hash),
            "sender": list(self.sender_pk),
            "nonce": self.nonce,
            "timestamp": self.timestamp,
            "recipient": self.recipient.to_json(),
            "payload": self.payload.to_json(),
            "signature": list(self.signature),
            "gas_limit": self.gas_limit,
            "gas_price": self.gas_price,
            "priority": self.priority,
            "metadata": self.metadata,
            "chain_id": self.chain_id,
        }
