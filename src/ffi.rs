use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint};
use std::ptr;
use std::sync::{Mutex, OnceLock};
use std::path::PathBuf;

use crate::sdk::BaaLSSdk;
use crate::types::{Transaction, PublicKey, Account, ContractId};

static SDK_INSTANCE: OnceLock<Mutex<BaaLSSdk>> = OnceLock::new();

// ─── Helpers ───

unsafe fn c_str_to_path(ptr: *const c_char) -> Option<PathBuf> {
    if ptr.is_null() { return None; }
    CStr::from_ptr(ptr).to_str().ok().map(PathBuf::from)
}

unsafe fn with_sdk<F, T>(f: F) -> Result<T, c_uint>
where F: FnOnce(&BaaLSSdk) -> Result<T, crate::sdk::SdkError>
{
    if let Some(sdk) = SDK_INSTANCE.get() {
        let sdk = sdk.lock().unwrap();
        f(&sdk).map_err(|_| 1u32)
    } else {
        Err(2u32)
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
pub extern "C" fn baals_sdk_init(data_dir: *const c_char) -> c_uint {
    let dir = match unsafe { c_str_to_path(data_dir) } {
        Some(d) => d,
        None => return 1,
    };
    match BaaLSSdk::new(dir) {
        Ok(sdk) => {
            let _ = SDK_INSTANCE.set(Mutex::new(sdk));
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
        with_sdk(|s| s.get_chain_state())
            .map(|cs| json_or_null(&cs))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_get_block_by_height(height: u64) -> *mut c_char {
    unsafe {
        with_sdk(|s| s.get_block_by_height(height))
            .ok().flatten()
            .map(|b| json_or_null(&b))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_get_transaction(hash_ptr: *const u8) -> *mut c_char {
    if hash_ptr.is_null() { return ptr::null_mut(); }
    let hash = unsafe { std::slice::from_raw_parts(hash_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(hash);
    unsafe {
        with_sdk(|s| s.get_transaction(&arr))
            .ok().flatten()
            .map(|tx| json_or_null(&tx))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_get_account(pubkey_ptr: *const u8) -> *mut c_char {
    if pubkey_ptr.is_null() { return ptr::null_mut(); }
    let pk_bytes = unsafe { std::slice::from_raw_parts(pubkey_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(pk_bytes);
    let pk = match PublicKey::from_bytes(&arr) { Ok(p) => p, Err(_) => return ptr::null_mut() };
    unsafe {
        with_sdk(|s| s.get_account(&pk))
            .ok().flatten()
            .map(|acct| json_or_null(&acct))
            .unwrap_or(ptr::null_mut())
    }
}

// ─── Transactions ───

#[no_mangle]
pub extern "C" fn baals_sdk_submit_tx(tx_json: *const c_char) -> c_uint {
    let json = match unsafe { c_str_to_path(tx_json) } {
        Some(p) => p.to_string_lossy().to_string(),
        None => return 1,
    };
    let tx: Transaction = match serde_json::from_str(&json) {
        Ok(t) => t,
        Err(_) => return 1,
    };
    unsafe { with_sdk(|s| s.submit_transaction(tx)).map(|_| 0).unwrap_or_else(|e| e) }
}

#[no_mangle]
pub extern "C" fn baals_sdk_create_account(pubkey_ptr: *const u8, balance: u64) -> c_uint {
    if pubkey_ptr.is_null() { return 1; }
    let pk_bytes = unsafe { std::slice::from_raw_parts(pubkey_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(pk_bytes);
    let pk = match PublicKey::from_bytes(&arr) { Ok(p) => p, Err(_) => return 1 };
    let account = Account::Wallet { balance, nonce: 0 };
    unsafe { with_sdk(|s| s.create_account(&pk, account)).map(|_| 0).unwrap_or_else(|e| e) }
}

// ─── Contracts ───

#[no_mangle]
pub extern "C" fn baals_sdk_deploy_contract(
    deployer_ptr: *const u8,
    wasm_ptr: *const u8,
    wasm_len: c_uint,
    init_payload_ptr: *const u8,
    init_payload_len: c_uint,
    gas_limit: u64,
) -> *mut c_char {
    if deployer_ptr.is_null() || wasm_ptr.is_null() { return ptr::null_mut(); }
    let pk_bytes = unsafe { std::slice::from_raw_parts(deployer_ptr, 32) };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(pk_bytes);
    let deployer = match PublicKey::from_bytes(&arr) { Ok(p) => p, Err(_) => return ptr::null_mut() };
    let wasm = unsafe { std::slice::from_raw_parts(wasm_ptr, wasm_len as usize) };
    let init_payload = if init_payload_ptr.is_null() || init_payload_len == 0 {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts(init_payload_ptr, init_payload_len as usize) })
    };
    unsafe {
        with_sdk(|s| s.deploy_contract(&deployer, wasm, init_payload, gas_limit))
            .map(|cid| json_or_null(&serde_json::json!({"contract_id": hex::encode(cid.to_bytes())})))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_call_contract(
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
    let pk_bytes = unsafe { std::slice::from_raw_parts(caller_ptr, 32) };
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(pk_bytes);
    let caller = match PublicKey::from_bytes(&pk_arr) { Ok(p) => p, Err(_) => return ptr::null_mut() };

    let cid_bytes = unsafe { std::slice::from_raw_parts(contract_id_ptr, 32) };
    let mut cid_arr = [0u8; 32];
    cid_arr.copy_from_slice(cid_bytes);
    let cid = ContractId::from_bytes(&cid_arr);

    let method_str = unsafe { CStr::from_ptr(method) }.to_string_lossy().to_string();
    let args = unsafe { std::slice::from_raw_parts(args_ptr, args_len as usize) };

    unsafe {
        with_sdk(|s| s.call_contract(&caller, &cid, &method_str, args, Some(value)))
            .map(|result| json_or_null(&serde_json::json!({"result_hex": hex::encode(&result)})))
            .unwrap_or(ptr::null_mut())
    }
}

#[no_mangle]
pub extern "C" fn baals_sdk_query_contract(
    contract_id_ptr: *const u8,
    method: *const c_char,
    payload_ptr: *const u8,
    payload_len: c_uint,
) -> *mut c_char {
    if contract_id_ptr.is_null() || method.is_null() { return ptr::null_mut(); }
    let cid_bytes = unsafe { std::slice::from_raw_parts(contract_id_ptr, 32) };
    let mut cid_arr = [0u8; 32];
    cid_arr.copy_from_slice(cid_bytes);
    let cid = ContractId::from_bytes(&cid_arr);
    let method_str = unsafe { CStr::from_ptr(method) }.to_string_lossy().to_string();
    let payload = if payload_ptr.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(payload_ptr, payload_len as usize) } };
    unsafe {
        with_sdk(|s| s.query_contract(&cid, &method_str, payload))
            .map(|result| json_or_null(&serde_json::json!({"result_hex": hex::encode(&result)})))
            .unwrap_or(ptr::null_mut())
    }
}

// ─── Memory Management ───

#[no_mangle]
pub extern "C" fn baals_sdk_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)); }
    }
}
