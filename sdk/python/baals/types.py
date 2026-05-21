"""BaaLS transaction types and bincode serialization helpers.

Matches Rust's bincode 1.x little-endian format exactly so that
SHA-256 transaction hashes computed in Python are identical to those
produced by the Rust node.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass
from typing import Union


# ---------------------------------------------------------------------------
# Bincode primitives
# ---------------------------------------------------------------------------

def _u8(v: int) -> bytes:
    return struct.pack("<B", v)

def _u32(v: int) -> bytes:
    return struct.pack("<I", v)

def _u64(v: int) -> bytes:
    return struct.pack("<Q", v)

def _bytes(v: bytes) -> bytes:
    """Vec<u8> / &[u8] — length-prefixed."""
    return _u64(len(v)) + v

def _string(v: str) -> bytes:
    """Rust String — length-prefixed UTF-8."""
    encoded = v.encode("utf-8")
    return _u64(len(encoded)) + encoded

def _option(v, serializer) -> bytes:
    """Option<T> — 0u8 for None, 1u8 + T for Some."""
    if v is None:
        return _u8(0)
    return _u8(1) + serializer(v)

def _btreemap_str_str(m: dict[str, str]) -> bytes:
    """BTreeMap<String, String> — sorted by key."""
    buf = _u64(len(m))
    for k in sorted(m):
        buf += _string(k) + _string(m[k])
    return buf


# ---------------------------------------------------------------------------
# Address
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class _Wallet:
    pubkey: bytes  # 32 bytes

    def bincode(self) -> bytes:
        # PublicKey custom Serialize: serialize_bytes(self.to_bytes())
        return _u32(0) + _bytes(self.pubkey)

    def to_json(self) -> dict:
        return {"Wallet": list(self.pubkey)}


@dataclass(frozen=True)
class _Contract:
    contract_id: bytes  # 32 bytes

    def bincode(self) -> bytes:
        # ContractId has #[derive(Serialize)] with id: [u8; 32] → raw 32 bytes
        return _u32(1) + self.contract_id

    def to_json(self) -> dict:
        return {"Contract": {"id": list(self.contract_id)}}


class Address:
    @staticmethod
    def wallet(pubkey: bytes | str) -> _Wallet:
        if isinstance(pubkey, str):
            pubkey = bytes.fromhex(pubkey)
        assert len(pubkey) == 32
        return _Wallet(pubkey)

    @staticmethod
    def contract(contract_id: bytes | str) -> _Contract:
        if isinstance(contract_id, str):
            contract_id = bytes.fromhex(contract_id)
        assert len(contract_id) == 32
        return _Contract(contract_id)


AddressType = Union[_Wallet, _Contract]


# ---------------------------------------------------------------------------
# TransactionPayload
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class _Transfer:
    amount: int

    def bincode(self) -> bytes:
        return _u32(0) + _u64(self.amount)

    def to_json(self) -> dict:
        return {"Transfer": {"amount": self.amount}}


@dataclass(frozen=True)
class _ContractDeploy:
    wasm_bytes: bytes
    init_payload: bytes | None = None

    def bincode(self) -> bytes:
        buf = _u32(1) + _bytes(self.wasm_bytes)
        buf += _option(self.init_payload, _bytes)
        return buf

    def to_json(self) -> dict:
        d: dict = {"wasm_bytes": list(self.wasm_bytes)}
        if self.init_payload is not None:
            d["init_payload"] = list(self.init_payload)
        else:
            d["init_payload"] = None
        return {"ContractDeploy": d}


@dataclass(frozen=True)
class _ContractCall:
    method: str
    args: list[bytes]
    value: int | None = None

    def bincode(self) -> bytes:
        buf = _u32(2) + _string(self.method)
        buf += _u64(len(self.args))
        for arg in self.args:
            buf += _bytes(arg)
        buf += _option(self.value, _u64)
        return buf

    def to_json(self) -> dict:
        return {
            "ContractCall": {
                "method": self.method,
                "args": [list(a) for a in self.args],
                "value": self.value,
            }
        }


@dataclass(frozen=True)
class _Data:
    data: bytes

    def bincode(self) -> bytes:
        return _u32(3) + _bytes(self.data)

    def to_json(self) -> dict:
        return {"Data": {"data": list(self.data)}}


@dataclass(frozen=True)
class _ValidatorSetChange:
    added: list[bytes]     # list of 32-byte pubkeys
    removed: list[bytes]   # list of 32-byte pubkeys
    effective_height: int

    def bincode(self) -> bytes:
        buf = _u32(4)
        buf += _u64(len(self.added))
        for pk in self.added:
            buf += _bytes(pk)  # PublicKey uses serialize_bytes
        buf += _u64(len(self.removed))
        for pk in self.removed:
            buf += _bytes(pk)
        buf += _u64(self.effective_height)
        return buf

    def to_json(self) -> dict:
        return {
            "ValidatorSetChange": {
                "added": [list(pk) for pk in self.added],
                "removed": [list(pk) for pk in self.removed],
                "effective_height": self.effective_height,
            }
        }


class TransactionPayload:
    @staticmethod
    def transfer(amount: int) -> _Transfer:
        return _Transfer(amount)

    @staticmethod
    def contract_deploy(wasm_bytes: bytes, init_payload: bytes | None = None) -> _ContractDeploy:
        return _ContractDeploy(wasm_bytes, init_payload)

    @staticmethod
    def contract_call(method: str, args: list[bytes] | None = None, value: int | None = None) -> _ContractCall:
        return _ContractCall(method, args or [], value)

    @staticmethod
    def data(data: bytes) -> _Data:
        return _Data(data)

    @staticmethod
    def validator_set_change(
        added: list[bytes], removed: list[bytes], effective_height: int
    ) -> _ValidatorSetChange:
        return _ValidatorSetChange(added, removed, effective_height)


PayloadType = Union[_Transfer, _ContractDeploy, _ContractCall, _Data, _ValidatorSetChange]
