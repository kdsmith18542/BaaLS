ALL CODE MUST BE PRODUCTION GRADE AND ABSOLUTELY NO STUBS WITH OUT ASKING!!

Deep Dive Blueprint: BaaLS Ledger & State Transition Model
Purpose: To precisely define how BaaLS manages the chain of blocks, validates new blocks and transactions, and deterministically updates the blockchain's state. This blueprint covers the core immutable ledger and the mechanisms for state change.

Relationship to BaaLS Core:
This module (ledger.rs in the libchain crate) is central to the Runtime. The Runtime orchestrates when the Ledger processes blocks, and the Ledger relies on Storage to persist the state and Contracts to execute smart contract logic.

Core Principles:

Determinism: Every valid block, when processed, must lead to the exact same new chain state, regardless of when or where it's executed.

Immutability: Once a block is added to the chain, it cannot be changed. All state changes are a result of applying new blocks.

Audibility: The entire history of state changes can be replayed by processing blocks from the genesis.

Simplicity: Favor straightforward, auditable logic over complex optimizations where security and determinism might be compromised.

1. Chain State Model
BaaLS will implement an Account-Based State Model (similar to Ethereum or NEAR). This is generally more flexible for smart contracts as it allows for direct manipulation of contract-specific data structures rather than tracking individual unspent transaction outputs.

Global ChainState: This is the root of the entire blockchain state. It's a snapshot of the ledger at a given block height.

Rust

pub struct ChainState {
    pub latest_block_hash: String,
    pub latest_block_index: u64,
    pub accounts_root_hash: String, // Merkle root of the accounts/contract state tree
    pub total_supply: u64, // (Optional) If BaaLS has a native token
    // ... other global chain metrics
}
Account Structure: Each address (PublicKey or ContractId) on the chain will correspond to an account.

Rust

pub enum Account {
    Wallet {
        balance: u64,
        nonce: u64, // To prevent replay attacks for transactions
    },
    Contract {
        code_hash: String, // Hash of the deployed WASM module
        storage_root_hash: String, // Merkle root of the contract's internal key-value storage
        nonce: u64, // To prevent replay attacks for contract calls
    },
}
Rationale for nonce: For both wallet and contract accounts, a transaction/call nonce is crucial to prevent replay attacks and ensure transactions are processed in the correct order for a given sender.

State Persistence (via Storage trait):

The Ledger module will interact with the Storage trait to read and write these Account and ChainState objects.

Key Design: Accounts and contract storage will be stored as key-value pairs in the underlying sled database.

accounts:<address_hash> -> Account serialized data

contract_storage:<contract_id>:<key> -> value (for internal contract state)

Merkle Trees (for accounts_root_hash and storage_root_hash):

To ensure deterministic and verifiable state, a sparse Merkle tree or similar data structure (e.g., a Merkle Patricia Trie, if more complex state queries are needed) will be used to represent the global account state and each contract's internal storage.

The accounts_root_hash in ChainState will be the root of the Merkle tree containing all account states.

Each Contract will have its own storage_root_hash representing its internal key-value store, enabling independent verification of contract state.

Benefit: This allows for light client proofs (proving a certain account state exists without downloading the entire chain) and ensures the immutability of the state snapshot tied to a block hash.

2. Block Validation Rules
Before a block can be added to the chain, the Ledger (in coordination with Consensus) must validate it.

Basic Block Header Validation:

index: Must be prev_block.index + 1.

prev_hash: Must match the hash of the current chain_state.latest_block_hash.

timestamp: Must be greater than prev_block.timestamp and within a reasonable tolerance of the current time (to prevent future blocks).

hash: Recompute the block hash based on its contents and ensure it matches block.hash.

nonce: Validated by the ConsensusEngine (e.g., for PoA, verify a signature or specific value).

Transaction Validation (within the block):

Signature Verification: Every transaction's signature must be cryptographically valid for its sender and payload.

Nonce Check: tx.nonce for the sender account must be account.nonce + 1. This prevents replay attacks and ensures sequential processing.

Sender Balance Check (if native token): If the transaction involves an amount, the sender must have sufficient balance.

Contract Deployment Validation:

If tx.payload is a WASM module intended for deployment:

Validate WASM bytecode (e.g., size limits, no disallowed opcodes for security, basic structural integrity).

Ensure recipient is a special "deployer" address or empty, indicating a new contract creation.

Contract Call Validation:

If tx.recipient is a deployed contract:

Verify the contract exists and tx.payload is a valid call to one of its exposed functions.

(Future) Validate gas limits for the call.

3. State Transition Logic
The core function where the ChainState is updated based on a validated block. This is a pure, deterministic function.

apply_block(block: &Block, current_state: &ChainState, storage: &mut dyn Storage, contract_engine: &dyn ContractEngine) -> Result<ChainState, StateTransitionError>

Load Current State: Retrieve the ChainState and all affected Account data from storage based on current_state.accounts_root_hash.

Process Transactions Sequentially: Iterate through block.transactions in the order they appear in the block.

For each tx:

Increment Sender Nonce: Update sender_account.nonce += 1.

If Native Token Transfer:

Decrement sender_account.balance by tx.amount.

Increment recipient_account.balance by tx.amount.

If Contract Deployment:

Generate a new unique ContractId (e.g., hash of deployer address + nonce + WASM code hash).

Create a new Account::Contract entry with code_hash and an initial empty storage_root_hash.

Persist the WASM bytecode (e.g., contract_code:<contract_id> -> wasm_bytes).

Call contract_engine.deploy_contract() to register and initialize the contract's internal state.

If Contract Call:

Retrieve recipient_contract_account from storage.

Call contract_engine.execute_contract_call(contract_id, sender, payload, storage):

The ContractEngine will load the WASM module.

Execute the specific function within the WASM module.

The WASM contract will use BaaLS-defined WASI host functions to read/write its own portion of the storage (e.g., baals_storage_read, baals_storage_write).

The ContractEngine will return the result of the call and any events emitted.

Handle returned ContractExecutionResult (e.g., success, error, gas used).

Error Handling: If any transaction fails (e.g., insufficient balance, invalid contract call, WASM runtime error), the entire block processing fails, and the state is not updated (atomicity).

Update Root Hashes: After all transactions are successfully processed:

Recalculate the accounts_root_hash based on all modified accounts.

(If applicable) Recalculate any other global Merkle roots.

Create New ChainState: Construct the new ChainState object with the updated hashes, latest_block_hash, and latest_block_index.

Persist Changes: The Runtime or Ledger will then commit the new ChainState and any updated Account data to Storage.

4. Cryptographic Hashing Rules
Block Hashing:

A block's hash is calculated by hashing a canonical representation of its header fields and the Merkle root of its transactions.

Algorithm: SHA256 (or Blake3 for faster performance in Rust environments).

Canonicalization: All fields must be deterministically serialized (e.g., using a fixed-size integer representation, sorted maps for metadata).

Transaction Hashing:

A transaction's ID is its hash, calculated by hashing a canonical representation of all its fields (sender, recipient, amount, payload, metadata).

Algorithm: SHA256 (or Blake3).

Account State Hashing:

The accounts_root_hash and storage_root_hash will be derived from a Merkle tree implementation that uses a consistent hashing algorithm (e.g., SHA256) for all its nodes.

5. Exception Handling & Rollbacks
Atomic Block Processing: The apply_block function should be atomic. If any step fails (e.g., a contract call reverts, an invalid transaction is found during re-execution), the entire block is considered invalid, and no state changes are committed for that block. This ensures the ledger never enters an inconsistent state.

Error Types: Define a clear hierarchy of LedgerError and StateTransitionError to communicate specific failures back to the Runtime. 