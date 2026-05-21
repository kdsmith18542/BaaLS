use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::StorageBackend;
use crate::sdk::BaaLSSdk;
use crate::types::{Account, ContractId, PublicKey, Transaction};

static SDK_INSTANCE: OnceLock<Mutex<BaaLSSdk>> = OnceLock::new();
static SDK_SHUTDOWN: AtomicBool = AtomicBool::new(false);

// ─── Helpers ───

/// # Safety
///
/// `ptr` must be null or a valid, null-terminated C string.
unsafe fn c_str_to_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string())
}

/// Returns the pathbuf if the pointer is non-null and valid UTF-8.
unsafe fn c_str_to_path(ptr: *const c_char) -> Option<PathBuf> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(PathBuf::from)
}

/// # Safety
///
/// The global SDK instance must have been initialized via `baals_sdk_init`.
/// Callers must ensure no concurrent init/access race.
unsafe fn with_sdk<F, T>(f: F) -> Result<T, c_uint>
where
    F: FnOnce(&BaaLSSdk) -> Result<T, crate::sdk::SdkError>,
{
    if SDK_SHUTDOWN.load(Ordering::Acquire) {
        return Err(6u32); // SDK shut down
    }
    if let Some(sdk_mutex) = SDK_INSTANCE.get() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let sdk = sdk_mutex.lock().map_err(|_| 4u32)?;
            f(&sdk).map_err(|_| 1u32)
        }));

        match result {
            Ok(inner_res) => inner_res,
            Err(_) => Err(3u32), // Panic occurred
        }
    } else {
        Err(2u32) // Not initialized
    }
}

fn json_or_null<T: serde::Serialize>(val: &T) -> *mut c_char {
    serde_json::to_string(val)
        .ok()
        .and_then(|s| CString::new(s).ok())
        .map(|cs| cs.into_raw())
        .unwrap_or(ptr::null_mut())
}

// ─── Init / Lifecycle ───

#[no_mangle]
/// # Safety
///
/// `data_dir` must be null or a valid, null-terminated C string path.
/// `backend` must be null or a valid, null-terminated C string ("sled" or "redb").
/// Should be called exactly once before any other SDK function.
/// Returns: 0 = OK, 1 = invalid input, 2 = init failed, 5 = already initialized
pub unsafe extern "C" fn baals_sdk_init(data_dir: *const c_char, backend: *const c_char) -> c_uint {
    let dir = match unsafe { c_str_to_path(data_dir) } {
        Some(d) => d,
        None => return 1,
    };
    let backend = match unsafe { c_str_to_str(backend) } {
        Some(s) if s.eq_ignore_ascii_case("redb") => StorageBackend::Redb,
        _ => StorageBackend::Sled,
    };
    match BaaLSSdk::with_backend(dir, Some(backend)) {
        Ok(sdk) => {
            if SDK_INSTANCE.set(Mutex::new(sdk)).is_err() {
                return 5; // BAALS_ERR_ALREADY_INITIALIZED
            }
            SDK_SHUTDOWN.store(false, Ordering::SeqCst);
            0 // BAALS_OK
        }
        Err(_) => 2,
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_start() -> c_uint {
    match unsafe { with_sdk(|s| s.start()) } {
        Ok(()) => 0,
        Err(e) => e,
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_stop() -> c_uint {
    match unsafe { with_sdk(|s| s.stop()) } {
        Ok(()) => 0,
        Err(e) => e,
    }
}

// ─── Query ───

#[no_mangle]
pub extern "C" fn baals_sdk_chain_state_json() -> *mut c_char {
    unsafe {
        with_sdk(|s| s.get_chain_state()).map(|cs| json_or_null(&cs)).unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_get_block_by_height(height: u64) -> *mut c_char {
    unsafe {
        with_sdk(|s| s.get_block_by_height(height))
            .ok()
            .flatten()
            .map(|b| json_or_null(&b))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
/// # Safety
///
/// `hash_ptr` must point to a valid 32-byte array (or be null, in which case null is returned).
pub unsafe extern "C" fn baals_sdk_get_transaction(hash_ptr: *const u8) -> *mut c_char {
    if hash_ptr.is_null() {
        return ptr::null_mut();
    }
    debug_assert!(!hash_ptr.is_null(), "hash_ptr must be non-null and point to 32 valid bytes");
    let hash = unsafe { std::slice::from_raw_parts(hash_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(hash);
    unsafe {
        with_sdk(|s| s.get_transaction(&arr))
            .ok()
            .flatten()
            .map(|tx| json_or_null(&tx))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
/// # Safety
///
/// `pubkey_ptr` must point to a valid 32-byte Ed25519 public key (or be null, in which case null is returned).
pub unsafe extern "C" fn baals_sdk_get_account(pubkey_ptr: *const u8) -> *mut c_char {
    if pubkey_ptr.is_null() {
        return ptr::null_mut();
    }
    debug_assert!(!pubkey_ptr.is_null(), "pubkey_ptr must point to 32 valid bytes");
    let pk_bytes = unsafe { std::slice::from_raw_parts(pubkey_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(pk_bytes);
    let pk = match PublicKey::from_bytes(&arr) {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };
    unsafe {
        with_sdk(|s| s.get_account(&pk))
            .ok()
            .flatten()
            .map(|acct| json_or_null(&acct))
            .unwrap_or(ptr::null_mut())
    }
}

// ─── Transactions ───

#[no_mangle]
/// # Safety
///
/// `tx_json` must be a valid, null-terminated JSON string.
pub unsafe extern "C" fn baals_sdk_submit_tx(tx_json: *const c_char) -> c_uint {
    let json = match unsafe { c_str_to_str(tx_json) } {
        Some(s) => s,
        None => return 1,
    };
    let tx: Transaction = match serde_json::from_str(&json) {
        Ok(t) => t,
        Err(_) => return 1,
    };
    unsafe { with_sdk(|s| s.submit_transaction(tx)).map(|_| 0).unwrap_or_else(|e| e) }
}

#[no_mangle]
/// # Safety
///
/// `pubkey_ptr` must point to a valid 32-byte Ed25519 public key (or be null).
pub unsafe extern "C" fn baals_sdk_create_account(pubkey_ptr: *const u8, balance: u64) -> c_uint {
    if pubkey_ptr.is_null() {
        return 1;
    }
    debug_assert!(!pubkey_ptr.is_null(), "pubkey_ptr must point to 32 valid bytes");
    let pk_bytes = unsafe { std::slice::from_raw_parts(pubkey_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(pk_bytes);
    let pk = match PublicKey::from_bytes(&arr) {
        Ok(p) => p,
        Err(_) => return 1,
    };
    let account = Account::Wallet { balance, nonce: 0 };
    unsafe { with_sdk(|s| s.create_account(&pk, account)).map(|_| 0).unwrap_or_else(|e| e) }
}

// ─── Contracts ───

#[no_mangle]
/// # Safety
///
/// All pointer arguments must be valid for reads of the specified length during the call.
/// `deployer_ptr` must point to 32 bytes. `wasm_ptr` must point to `wasm_len` bytes.
/// `init_payload_ptr` may be null (if `init_payload_len` is 0) or point to `init_payload_len` bytes.
pub unsafe extern "C" fn baals_sdk_deploy_contract(
    deployer_ptr: *const u8,
    wasm_ptr: *const u8,
    wasm_len: c_uint,
    init_payload_ptr: *const u8,
    init_payload_len: c_uint,
    gas_limit: u64,
) -> *mut c_char {
    if deployer_ptr.is_null() || wasm_ptr.is_null() {
        return ptr::null_mut();
    }
    debug_assert!(!deployer_ptr.is_null(), "deployer_ptr must point to 32 valid bytes");
    let pk_bytes = unsafe { std::slice::from_raw_parts(deployer_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(pk_bytes);
    let deployer = match PublicKey::from_bytes(&arr) {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };
    let wasm = unsafe { std::slice::from_raw_parts(wasm_ptr, wasm_len as usize) };
    let init_payload = if init_payload_ptr.is_null() || init_payload_len == 0 {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts(init_payload_ptr, init_payload_len as usize) })
    };
    unsafe {
        with_sdk(|s| s.deploy_contract(&deployer, wasm, init_payload, gas_limit))
            .map(|cid| {
                json_or_null(&serde_json::json!({"contract_id": hex::encode(cid.to_bytes())}))
            })
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
/// # Safety
///
/// All pointer arguments must be valid for reads. `caller_ptr` and `contract_id_ptr`
/// must point to 32 bytes each. `method` must be a null-terminated string.
/// `args_ptr` may be null if `args_len` is 0.
pub unsafe extern "C" fn baals_sdk_call_contract(
    caller_ptr: *const u8,
    contract_id_ptr: *const u8,
    method: *const c_char,
    args_ptr: *const u8,
    args_len: c_uint,
    value: u64,
) -> *mut c_char {
    if caller_ptr.is_null() || contract_id_ptr.is_null() || method.is_null() {
        return ptr::null_mut();
    }
    debug_assert!(!caller_ptr.is_null(), "caller_ptr must point to 32 valid bytes");
    debug_assert!(!contract_id_ptr.is_null(), "contract_id_ptr must point to 32 valid bytes");
    let pk_bytes = unsafe { std::slice::from_raw_parts(caller_ptr, 32) };
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(pk_bytes);
    let caller = match PublicKey::from_bytes(&pk_arr) {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };

    let cid_bytes = unsafe { std::slice::from_raw_parts(contract_id_ptr, 32) };
    let mut cid_arr = [0u8; 32];
    cid_arr.copy_from_slice(cid_bytes);
    let cid = ContractId::from_bytes(&cid_arr);

    let method_str = unsafe { CStr::from_ptr(method) }.to_string_lossy().to_string();
    let args: Vec<Vec<u8>> = if args_ptr.is_null() || args_len == 0 {
        Vec::new()
    } else {
        let args_bytes = unsafe { std::slice::from_raw_parts(args_ptr, args_len as usize) };
        bincode::deserialize(args_bytes).unwrap_or_else(|_| vec![args_bytes.to_vec()])
    };

    unsafe {
        with_sdk(|s| s.call_contract(&caller, &cid, &method_str, &args, Some(value)))
            .map(|result| json_or_null(&serde_json::json!({"result_hex": hex::encode(&result)})))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
/// # Safety
///
/// `contract_id_ptr` must point to 32 valid bytes. `method` must be a null-terminated string.
/// `payload_ptr` may be null if `payload_len` is 0.
pub unsafe extern "C" fn baals_sdk_query_contract(
    contract_id_ptr: *const u8,
    method: *const c_char,
    payload_ptr: *const u8,
    payload_len: c_uint,
) -> *mut c_char {
    if contract_id_ptr.is_null() || method.is_null() {
        return ptr::null_mut();
    }
    debug_assert!(!contract_id_ptr.is_null(), "contract_id_ptr must point to 32 valid bytes");
    let cid_bytes = unsafe { std::slice::from_raw_parts(contract_id_ptr, 32) };
    let mut cid_arr = [0u8; 32];
    cid_arr.copy_from_slice(cid_bytes);
    let cid = ContractId::from_bytes(&cid_arr);
    let method_str = unsafe { CStr::from_ptr(method) }.to_string_lossy().to_string();
    let payload = if payload_ptr.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(payload_ptr, payload_len as usize) }
    };
    unsafe {
        with_sdk(|s| s.query_contract(&cid, &method_str, payload))
            .map(|result| json_or_null(&serde_json::json!({"result_hex": hex::encode(&result)})))
            .unwrap_or(ptr::null_mut())
    }
}

// ─── P2P / Sync ───

#[no_mangle]
/// # Safety
///
/// `address` must be a valid, null-terminated C string.
pub unsafe extern "C" fn baals_sdk_add_peer(address: *const c_char) -> c_uint {
    let addr = match unsafe { c_str_to_str(address) } {
        Some(s) => s,
        None => return 1,
    };
    unsafe { with_sdk(|s| s.add_peer(&addr)).map(|_| 0).unwrap_or_else(|e| e) }
}

#[no_mangle]
/// # Safety
///
/// `address` must be a valid, null-terminated C string.
pub unsafe extern "C" fn baals_sdk_remove_peer(address: *const c_char) -> c_uint {
    let addr = match unsafe { c_str_to_str(address) } {
        Some(s) => s,
        None => return 1,
    };
    unsafe { with_sdk(|s| s.remove_peer(&addr)).map(|_| 0).unwrap_or_else(|e| e) }
}

#[no_mangle]
pub extern "C" fn baals_sdk_trigger_sync() -> c_uint {
    match unsafe { with_sdk(|s| s.trigger_sync()) } {
        Ok(()) => 0,
        Err(e) => e,
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_known_peers_json() -> *mut c_char {
    unsafe {
        with_sdk(|s| Ok::<_, crate::sdk::SdkError>(s.known_peers()))
            .map(|peers| json_or_null(&peers))
            .unwrap_or(ptr::null_mut())
    }
}

// ─── Shutdown ───

#[no_mangle]
pub extern "C" fn baals_sdk_shutdown() -> c_uint {
    SDK_SHUTDOWN.store(true, Ordering::Release);
    match unsafe { with_sdk(|s| s.stop()) } {
        Ok(()) => 0,
        Err(e) => e,
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_version() -> *mut c_char {
    let version = env!("CARGO_PKG_VERSION");
    CString::new(version).ok().map(|cs| cs.into_raw()).unwrap_or(ptr::null_mut())
}

// ─── Memory Management ───

#[no_mangle]
/// # Safety
///
/// `s` must have been allocated by a previous `baals_sdk_*` call that returned a `*mut c_char`.
/// Double-free or use-after-free will cause undefined behavior.
pub unsafe extern "C" fn baals_sdk_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}
