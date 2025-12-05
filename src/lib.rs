// src/lib.rs

//! # BaaLS - Blockchain as a Local Service
//!
//! BaaLS is a lightweight, embeddable blockchain engine written in Rust.
//! It provides local-first blockchain functionality with optional peer-to-peer synchronization.
//!
//! ## Core Modules
//!
//! - [`types`]: Core data structures (Block, Transaction, etc.)
//! - [`storage`]: Persistent storage layer using sled
//! - [`ledger`]: Block validation and state transition logic
//! - [`consensus`]: Consensus engine (Proof-of-Authority)
//! - [`runtime`]: Main runtime orchestrator
//! - [`contracts`]: WASM smart contract execution engine
//! - [`sync`]: Optional peer-to-peer synchronization

pub mod types;
pub mod storage;
pub mod ledger;
pub mod consensus;
pub mod runtime;
pub mod sync;
pub mod contracts; 