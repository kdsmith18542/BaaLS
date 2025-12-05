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

pub mod consensus;
pub mod contracts;
pub mod ledger;
pub mod runtime;
pub mod storage;
pub mod sync;
pub mod types;
