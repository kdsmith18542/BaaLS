//! # BaaLS: Blockchain as a Local Service
//!
//! BaaLS is an embeddable, local-first blockchain ledger written in Rust. It provides immutable, auditable, and deterministic data storage for decentralized applications, with a focus on simplicity, locality, and extensibility. BaaLS aims to be the "SQLite of blockchains".
//!
//! ## Core Modules
//! - Ledger: Block validation, state transition, chain integrity
//! - Storage: Sled-based key-value store with Merkle root support
//! - Consensus: Pluggable consensus (default: PoA)
//! - Contracts: WASM smart contract runtime
//! - Runtime: Central coordinator and public API
//! - Sync: Optional peer-to-peer synchronization
//! - CLI & SDK: Command-line tools and Rust SDK

pub mod any_storage;
/// Configuration system (TOML-based)
pub mod config;
/// Consensus engine trait and default PoA implementation
pub mod consensus;
/// WASM smart contract engine and host functions
pub mod contracts;
/// Foreign Function Interface (C bindings)
pub mod ffi;
/// Secure encrypted keystore for private key management
pub mod keystore;
/// Ledger logic: block validation, state transition, Merkle root
pub mod ledger;
/// Performance metrics collection and reporting
pub mod metrics;
pub mod redb_storage;
/// Central runtime coordinator and public API
pub mod runtime;
/// Rust SDK for programmatic interaction
pub mod sdk;
/// Storage abstraction, Sled-based and Redb-based implementations
pub mod storage;
/// Optional peer-to-peer sync layer
pub mod sync;
/// Core types and data structures (accounts, blocks, transactions, etc.)
pub mod types;

pub use any_storage::*;
pub use config::*;
pub use consensus::*;
pub use contracts::*;
pub use keystore::*;
pub use ledger::*;
pub use metrics::*;
pub use redb_storage::*;
pub use runtime::*;
pub use sdk::*;
pub use storage::*;
pub use sync::*;
pub use types::*;
