/// BaaLS Oracle WASM Contract — Phase K
///
/// Deployed to the BaaLS ledger as the on-chain oracle state machine.
/// Handles two operations:
///   - `record(ptr, len) -> i32`  : store a signed OracleAttestation in contract storage
///   - `query(ptr, len) -> i32`   : read an attestation by chain_id + address, write to memory
///
/// BaaLS calls these as: fn method(ptr: i32, len: i32) -> i32
///   - ptr/len point to bincode-serialized Vec<Vec<u8>> at WASM memory offset 0
///   - return value is bytes written to WASM memory at offset 0 (0 = ok, -1 = error)
///
/// Host functions available (imported from "env"):
///   baals_storage_write(key_ptr, key_len, value_ptr, value_len) -> i32
///   baals_storage_read(key_ptr, key_len, value_ptr, value_len_cap) -> i32
///   baals_get_block_index() -> u64

use serde::{Deserialize, Serialize};

// ─── Host Function Imports ───────────────────────────────────────────────────

extern "C" {
    fn baals_storage_write(
        key_ptr: i32,
        key_len: i32,
        value_ptr: i32,
        value_len: i32,
    ) -> i32;

    fn baals_storage_read(
        key_ptr: i32,
        key_len: i32,
        value_ptr: i32,
        value_len_cap: i32,
    ) -> i32;

    fn baals_get_block_index() -> u64;
}

// ─── Types ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct RecordInput {
    chain_id: String,
    address: String,
    /// Full JSON-serialized OracleAttestation from BaaLS node
    attestation_json: String,
}

#[derive(Serialize, Deserialize)]
struct QueryInput {
    chain_id: String,
    address: String,
}

// ─── Storage key helpers ──────────────────────────────────────────────────────

fn attestation_key(chain_id: &str, address: &str) -> Vec<u8> {
    format!("oracle:attest:{}:{}", chain_id, address).into_bytes()
}

fn chain_index_key(chain_id: &str) -> Vec<u8> {
    format!("oracle:index:{}", chain_id).into_bytes()
}

fn chains_key() -> &'static [u8] {
    b"oracle:chains"
}

// ─── Memory helpers ───────────────────────────────────────────────────────────

/// Read args from WASM memory at (ptr, len) and bincode-decode as Vec<Vec<u8>>.
fn read_args(ptr: i32, len: i32) -> Option<Vec<Vec<u8>>> {
    if len <= 0 || ptr < 0 {
        return None;
    }
    let mem = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    bincode::deserialize::<Vec<Vec<u8>>>(mem).ok()
}

/// Write bytes to WASM memory at offset 0, return number of bytes written.
///
/// BaaLS writes args at offset 0 before calling us, then reads `ret_val`
/// bytes from offset 0 after we return. In WASM32 linear memory, address 0
/// is valid (the host has pre-allocated at least 1 page). black_box makes
/// the address opaque to the compile-time null-pointer lint, which otherwise
/// fires on `0 as *mut u8` even though WASM has no OS-level null trap.
fn write_output(data: &[u8]) -> i32 {
    if data.is_empty() {
        return 0;
    }
    unsafe {
        let dst = std::hint::black_box(0usize) as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
    }
    data.len() as i32
}

// ─── Storage wrappers ─────────────────────────────────────────────────────────

fn storage_write(key: &[u8], value: &[u8]) -> bool {
    let ret = unsafe {
        baals_storage_write(
            key.as_ptr() as i32,
            key.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        )
    };
    ret >= 0
}

fn storage_read(key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 65536];
    let n = unsafe {
        baals_storage_read(
            key.as_ptr() as i32,
            key.len() as i32,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
        )
    };
    if n > 0 {
        buf.truncate(n as usize);
        Some(buf)
    } else {
        None
    }
}

// ─── Contract Methods ─────────────────────────────────────────────────────────

/// record(ptr, len) — store a BaaLS-signed OracleAttestation in contract storage.
///
/// Args[0]: JSON-encoded RecordInput { chain_id, address, attestation_json }
/// Returns: 0 on success written to memory, -1 on error.
#[no_mangle]
pub extern "C" fn record(ptr: i32, len: i32) -> i32 {
    let args = match read_args(ptr, len) {
        Some(a) => a,
        None => return -1,
    };

    let input_bytes = match args.first() {
        Some(b) => b,
        None => return -1,
    };

    let input: RecordInput = match serde_json::from_slice(input_bytes) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    if input.chain_id.is_empty() || input.address.is_empty() || input.attestation_json.is_empty() {
        return -1;
    }

    // Validate attestation_json is valid JSON before storing
    if serde_json::from_str::<serde_json::Value>(&input.attestation_json).is_err() {
        return -1;
    }

    // Write attestation
    let key = attestation_key(&input.chain_id, &input.address);
    if !storage_write(&key, input.attestation_json.as_bytes()) {
        return -1;
    }

    // Update chain address index
    let idx_key = chain_index_key(&input.chain_id);
    let mut addresses: Vec<String> = storage_read(&idx_key)
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    if !addresses.contains(&input.address) {
        addresses.push(input.address.clone());
        let idx_val = match serde_json::to_vec(&addresses) {
            Ok(v) => v,
            Err(_) => return -1,
        };
        if !storage_write(&idx_key, &idx_val) {
            return -1;
        }
    }

    // Update global chain registry for submitter discovery.
    let mut chains: Vec<String> = storage_read(chains_key())
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    if !chains.contains(&input.chain_id) {
        chains.push(input.chain_id.clone());
        let chains_val = match serde_json::to_vec(&chains) {
            Ok(v) => v,
            Err(_) => return -1,
        };
        if !storage_write(chains_key(), &chains_val) {
            return -1;
        }
    }

    // Write block index metadata
    let block_idx = unsafe { baals_get_block_index() };
    let meta_key = format!("oracle:meta:{}:{}", input.chain_id, input.address).into_bytes();
    let meta_val = block_idx.to_be_bytes();
    storage_write(&meta_key, &meta_val);

    let ok = b"ok";
    write_output(ok)
}

/// query(ptr, len) — retrieve a stored OracleAttestation by chain_id + address.
///
/// Args[0]: JSON-encoded QueryInput { chain_id, address }
/// Returns: bytes of attestation JSON written to memory at offset 0, or -1 if not found.
#[no_mangle]
pub extern "C" fn query(ptr: i32, len: i32) -> i32 {
    let args = match read_args(ptr, len) {
        Some(a) => a,
        None => return -1,
    };

    let input_bytes = match args.first() {
        Some(b) => b,
        None => return -1,
    };

    let input: QueryInput = match serde_json::from_slice(input_bytes) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let key = attestation_key(&input.chain_id, &input.address);
    match storage_read(&key) {
        Some(val) => write_output(&val),
        None => -1,
    }
}

/// list(ptr, len) — list all attested addresses for a chain.
///
/// Args[0]: chain_id as UTF-8 bytes
/// Returns: JSON array of address strings written to memory, or -1.
#[no_mangle]
pub extern "C" fn list(ptr: i32, len: i32) -> i32 {
    let args = match read_args(ptr, len) {
        Some(a) => a,
        None => return -1,
    };

    let chain_id_bytes = match args.first() {
        Some(b) => b,
        None => return -1,
    };
    let chain_id = match std::str::from_utf8(chain_id_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let idx_key = chain_index_key(chain_id);
    let addresses: Vec<String> = storage_read(&idx_key)
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();

    let json = match serde_json::to_vec(&addresses) {
        Ok(v) => v,
        Err(_) => return -1,
    };
    write_output(&json)
}
