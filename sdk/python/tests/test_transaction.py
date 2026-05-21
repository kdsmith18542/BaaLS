"""Cross-validate Python transaction hashing against Rust's deterministic test vector.

The expected values come from ``src/types.rs::test_deterministic_hash_vector``
which uses sender_sk=[1u8;32], recipient_sk=[2u8;32], nonce=42, timestamp=1700000000,
gas_limit=100000, gas_price=1, priority=0, chain_id=1, Transfer{amount:1000}, no metadata.
"""

from nacl.signing import SigningKey

from baals.types import Address, TransactionPayload
from baals.transaction import Transaction


SENDER_SK = bytes([1]) * 32
RECIPIENT_SK = bytes([2]) * 32

SENDER_PK = SigningKey(SENDER_SK).verify_key.encode()
RECIPIENT_PK = SigningKey(RECIPIENT_SK).verify_key.encode()

EXPECTED_SENDER_PK_HEX = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c"
EXPECTED_RECIPIENT_PK_HEX = "8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394"
EXPECTED_HASH_HEX = "616107ff2c9c85c227275c6e1c0c7cf3d7cfff3f0e479c8bd2e68706d3ec9cfc"


def test_derived_public_keys_match_rust():
    assert SENDER_PK.hex() == EXPECTED_SENDER_PK_HEX
    assert RECIPIENT_PK.hex() == EXPECTED_RECIPIENT_PK_HEX


def test_bincode_address_wallet():
    addr = Address.wallet(RECIPIENT_PK)
    expected = bytes.fromhex(
        "00000000"                                  # u32 tag = 0 (Wallet)
        "2000000000000000"                          # u64 = 32 (PublicKey length)
        "8139770ea87d175f56a35466c34c7ecccb8d8a91"  # pubkey bytes...
        "b4ee37a25df60f5b8fc9b394"
    )
    assert addr.bincode() == expected


def test_bincode_payload_transfer():
    payload = TransactionPayload.transfer(1000)
    expected = bytes.fromhex(
        "00000000"          # u32 tag = 0 (Transfer)
        "e803000000000000"  # u64 = 1000
    )
    assert payload.bincode() == expected


def test_transaction_hash_matches_rust():
    tx = Transaction(
        sender_pk=SENDER_PK,
        recipient=Address.wallet(RECIPIENT_PK),
        payload=TransactionPayload.transfer(1000),
        nonce=42,
        gas_limit=100_000,
        gas_price=1,
        priority=0,
        chain_id=1,
        timestamp=1700000000,
        metadata=None,
    )
    assert tx.calculate_hash().hex() == EXPECTED_HASH_HEX


def test_sign_and_json_roundtrip():
    tx = Transaction(
        sender_pk=SENDER_PK,
        recipient=Address.wallet(RECIPIENT_PK),
        payload=TransactionPayload.transfer(500),
        nonce=0,
        timestamp=1700000000,
    )
    tx.sign(SENDER_SK)
    j = tx.to_json()
    assert len(j["hash"]) == 32
    assert len(j["signature"]) == 64
    assert j["nonce"] == 0
    assert j["payload"] == {"Transfer": {"amount": 500}}
    assert j["recipient"] == {"Wallet": list(RECIPIENT_PK)}


def test_contract_call_payload_bincode():
    payload = TransactionPayload.contract_call("transfer", [b"\x01\x02"], value=100)
    bc = payload.bincode()
    assert bc[:4] == bytes.fromhex("02000000")  # tag = 2 (ContractCall)


def test_data_payload_bincode():
    payload = TransactionPayload.data(b"hello")
    bc = payload.bincode()
    assert bc[:4] == bytes.fromhex("03000000")  # tag = 3 (Data)
    assert bc[4:12] == bytes.fromhex("0500000000000000")  # len = 5
    assert bc[12:] == b"hello"
