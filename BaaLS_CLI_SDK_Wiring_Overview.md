Deep Dive Blueprint: BaaLS CLI & SDK Wiring Overview
Purpose: To define the external programmatic and command-line interfaces that allow users and applications to interact with a BaaLS instance. This blueprint emphasizes ease of use, broad language support, and secure access to the core BaaLS Runtime functionalities.

Relationship to BaaLS Core:
Both the CLI and SDKs act as clients to the core libchain crate and its Runtime module. They translate user commands or programmatic calls into specific function invocations within the BaaLS runtime, handling data serialization, deserialization, and error propagation across language boundaries where necessary.

Core Principles:

Ease of Use: Intuitive commands and clear APIs for both human and programmatic interaction.

Broad Language Support: Provide idiomatic SDKs for popular languages, leveraging FFI where native Rust integration isn't feasible.

Comprehensive Functionality: Expose all core BaaLS features, from node management to smart contract interaction.

Secure Access: Ensure private keys are handled securely and interactions with the BaaLS instance (especially for networked scenarios) can be authenticated.

Structured Output: Provide machine-readable (JSON) output options for easy integration into scripts and other applications.

1. CLI Tools (baals executable)
The baals command-line interface will be the primary tool for developers to interact with a BaaLS instance directly, manage its lifecycle, and perform quick operations. Built using a robust Rust CLI framework like clap.

Role: Direct interaction, scripting, node administration, development, and debugging.

Key Command Categories & Examples:

Node Management (baals node ...): For starting, stopping, and configuring a BaaLS instance.

baals node start [path/to/config.toml]

Starts a BaaLS node, optionally using a specified configuration file. Runs in foreground or daemonized.

baals node stop

Gracefully stops a running BaaLS node.

baals node status

Displays the current state of the node (running/stopped, latest block, mempool size, sync status).

baals node config init

Generates a default config.toml file.

baals node config set <key> <value>

Updates a specific configuration parameter.

Wallet Management (baals wallet ...): For keypair generation, import, and listing.

baals wallet create [--name <name>]

Generates a new cryptographic keypair and stores it securely (e.g., encrypted local keystore). Returns public key.

baals wallet list

Lists all managed public keys (addresses).

baals wallet import <private_key_hex>

Imports an existing private key.

baals wallet export <public_key> [--password <password>]

Exports an encrypted private key (with passphrase).

baals wallet sign <public_key> <message_hex>

Signs a raw message with a specified key.

Transaction Submission (baals tx ...): For constructing and submitting various transaction types.

baals tx transfer --sender <pubkey> --recipient <address> --amount <value> [--memo <string>]

Submits a native token transfer transaction.

baals tx deploy-contract --sender <pubkey> --wasm <path/to/wasm> [--init-args <json>] [--gas-limit <units>]

Deploys a WASM smart contract. The init-args JSON would be passed to the contract's initialization function.

baals tx call-contract --sender <pubkey> --contract-id <id> --method <name> --args <json> [--value <amount>] [--gas-limit <units>]

Calls a function on a deployed smart contract. args would be JSON and translated to the contract's expected ABI encoding. value for native token transfer with call.

baals tx data --sender <pubkey> --data <hex_or_string>

Submits a raw data transaction.

baals tx inspect <path/to/signed_tx_file>

Parses and displays details of a raw signed transaction file.

Chain Query (baals query ...): For retrieving data from the blockchain.

baals query head

Displays the latest block hash and height.

baals query block <hash_or_height>

Retrieves and displays a specific block's details (full content or summary).

baals query tx <tx_hash>

Retrieves and displays a specific transaction's details.

baals query account <address>

Displays the details of a wallet or contract account (balance, nonce, contract code hash).

baals query contract-state --contract-id <id> --key <hex_key>

Reads a specific key-value from a contract's local storage.

baals query contract-call --contract-id <id> --method <name> --args <json>

Executes a read-only query (simulated call) on a smart contract without changing state.

Development/Debugging (baals dev ...): Utilities for testing and development.

baals dev generate-keys [--count <n>]

Generates new keypairs for testing purposes.

baals dev simulate-contract --wasm <path/to/wasm> --method <name> --args <json> [--sender <pubkey>]

Runs a WASM module locally in the sandbox for testing without deploying to the chain. Displays output, gas usage, and simulated state changes.

baals dev validate-tx <path/to/raw_tx_file>

Validates a raw transaction file for format and signature.

Output Formats:

Default: Human-readable, nicely formatted tables or descriptive text.

--json flag: Output machine-readable JSON for all queries, enabling easy piping to other tools.

Configuration: CLI commands will use a hierarchical configuration system (CLI flags > Environment Variables > Config File > Default values).

2. SDKs (Software Development Kits)
SDKs provide the necessary libraries and tools for developers to integrate BaaLS functionalities directly into their applications (desktop, mobile, web backend). 