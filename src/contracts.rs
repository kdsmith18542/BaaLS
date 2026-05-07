use crate::storage::Storage;
use crate::types::{ContractId, PublicKey, TransactionSignature};
use ed25519_dalek::Signature as Ed25519Signature;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use wasmtime::{Caller, Config, Engine, Linker, Module, Store};

type InterContractCall = (Vec<u8>, Vec<u8>, Vec<Vec<u8>>, u64);

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("Storage error: {0}")]
    StorageError(#[from] crate::storage::StorageError),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Contract not found: {0}")]
    ContractNotFound(String),
    #[error("Invalid WASM: {0}")]
    InvalidWasm(String),
    #[error("WASM runtime error: {0}")]
    WasmRuntimeError(String),
    #[error("Gas limit exceeded")]
    GasLimitExceeded,
    #[error("Memory access error: {0}")]
    MemoryAccessError(String),
    #[error("Host function error: {0}")]
    HostFunctionError(String),
    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),
    #[error("Execution timeout")]
    ExecutionTimeout,
    #[error("Contract reverted: {0}")]
    Reverted(String),
    #[error("Reentrancy detected on contract {0}")]
    ReentrancyDetected(String),
    #[error("WASM bytecode validation failed: {0}")]
    BytecodeValidationFailed(String),
    #[error("Inter-contract call failed: {0}")]
    InterContractCallError(String),
}

pub trait ContractEngine: Send + Sync {
    fn deploy_contract(
        &self,
        deployer: &PublicKey,
        deployer_nonce: u64,
        wasm_bytes: &[u8],
        init_payload: Option<&[u8]>,
        storage: &dyn Storage,
        gas_limit: u64,
    ) -> Result<ContractId, ContractError>;

    #[allow(clippy::too_many_arguments)]
    fn call_contract(
        &self,
        caller: &PublicKey,
        contract_id: &ContractId,
        method_name: &str,
        args: &[Vec<u8>],
        value: Option<u64>,
        storage: &dyn Storage,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<Vec<u8>, ContractError>;

    fn query_contract(
        &self,
        contract_id: &ContractId,
        method_name: &str,
        payload: &[u8],
        storage: &dyn Storage,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<Vec<u8>, ContractError>;

    fn verify_contract(
        &self,
        wasm_bytes: &[u8],
        _contract_id: &ContractId,
    ) -> Result<(), ContractError> {
        let engine = Engine::default();
        Module::new(&engine, wasm_bytes).map_err(|e| ContractError::InvalidWasm(e.to_string()))?;
        Ok(())
    }

    fn estimate_gas_usage(
        &self,
        contract_id: &ContractId,
        method_name: &str,
        args: &[Vec<u8>],
        storage: &dyn Storage,
    ) -> Result<GasEstimate, ContractError>;

    fn get_contract_metrics(
        &self,
        contract_id: &ContractId,
    ) -> Result<ContractMetrics, ContractError>;

    fn set_resource_limits(
        &mut self,
        memory_limit: usize,
        table_limit: u32,
        stack_limit: u32,
    ) -> Result<(), ContractError>;
}

/// Abstraction over the WASM runtime execution layer.
/// Enables swapping wasmtime for other runtimes (wasmer, etc.) without
/// changing the ContractEngine logic.
pub trait WasmRuntime: Send + Sync {
    /// Execute a WASM contract method and return (result_data, gas_used, events).
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn execute_wasm_contract(
        &self,
        wasm_bytes: &[u8],
        method_name: &str,
        args: &[Vec<u8>],
        caller: &PublicKey,
        contract_id: &ContractId,
        storage: &dyn Storage,
        read_only: bool,
        gas_limit: u64,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<(Vec<u8>, u64, Vec<(Vec<u8>, Vec<u8>)>), ContractError>;

    /// Validate WASM bytecode before deployment (magic bytes, size, opcodes, memory limits).
    fn validate_wasm_module(&self, wasm_bytes: &[u8]) -> Result<(), ContractError>;

    /// Estimate gas usage for a contract method call.
    fn estimate_gas(
        &self,
        wasm_bytes: &[u8],
        method_name: &str,
        args: &[Vec<u8>],
    ) -> Result<u64, ContractError>;
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ContractExecutionResult {
    pub success: bool,
    pub output_data: Option<Vec<u8>>,
    pub gas_used: u64,
    pub error_message: Option<String>,
    pub execution_time: Duration,
    pub memory_used: usize,
    pub instructions_executed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub estimated_gas: u64,
    pub confidence_level: f64,
    pub execution_time_estimate: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMetrics {
    pub total_calls: u64,
    pub total_gas_used: u64,
    pub average_execution_time: Duration,
    pub success_rate: f64,
    pub last_called: Option<u64>,
    pub memory_usage_stats: MemoryUsageStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUsageStats {
    pub peak_memory: usize,
    pub average_memory: usize,
    pub memory_growth_rate: f64,
}

#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub memory_limit: usize,
    pub table_limit: u32,
    pub stack_limit: u32,
    pub execution_timeout: Duration,
    pub gas_limit: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_limit: 64 * 1024 * 1024, // 64MB
            table_limit: 10000,
            stack_limit: 1_048_576, // 1MB
            execution_timeout: Duration::from_secs(30),
            gas_limit: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractPermissions {
    pub storage_read: bool,
    pub storage_write: bool,
    pub storage_remove: bool,
    pub call_contracts: bool,
    pub emit_events: bool,
    pub get_block_info: bool,
    pub crypto_ops: bool,
    pub revert: bool,
}

impl Default for ContractPermissions {
    fn default() -> Self {
        Self::all()
    }
}

impl ContractPermissions {
    pub fn all() -> Self {
        Self {
            storage_read: true,
            storage_write: true,
            storage_remove: true,
            call_contracts: true,
            emit_events: true,
            get_block_info: true,
            crypto_ops: true,
            revert: true,
        }
    }

    pub fn restricted() -> Self {
        Self {
            storage_read: true,
            storage_write: false,
            storage_remove: false,
            call_contracts: false,
            emit_events: false,
            get_block_info: true,
            crypto_ops: false,
            revert: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAbi {
    pub methods: Vec<ContractMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMethod {
    pub name: String,
    pub arg_count: usize,
}

// ─── HostState: passed through wasmtime Caller ───

struct HostState {
    caller: PublicKey,
    contract_id: ContractId,
    contract_storage: HashMap<Vec<u8>, Vec<u8>>,
    storage: Box<dyn Storage>,
    read_only: bool,
    gas_used: u64,
    gas_limit: u64,
    block_index: u64,
    block_timestamp: u64,
    input_data: Vec<u8>,
    reverted: bool,
    events: Vec<(Vec<u8>, Vec<u8>)>,
    last_memory_size: usize,
    inter_contract_calls: Vec<InterContractCall>,
    inter_contract_results: Vec<Vec<u8>>,
    deleted_keys: Vec<Vec<u8>>,
    permissions: ContractPermissions,
}

impl HostState {
    fn charge_gas(&mut self, amount: u64) -> Result<(), ContractError> {
        self.gas_used += amount;
        if self.gas_used > self.gas_limit {
            Err(ContractError::GasLimitExceeded)
        } else {
            Ok(())
        }
    }

    fn charge_memory_growth(&mut self, current_memory_size: usize) -> Result<(), ContractError> {
        if current_memory_size > self.last_memory_size {
            let growth_bytes = current_memory_size - self.last_memory_size;
            let growth_pages = growth_bytes.div_ceil(65536).max(1) as u64;
            self.charge_gas(growth_pages * 100)?;
            self.last_memory_size = current_memory_size;
        }
        Ok(())
    }
}

// ─── Contract Engine ───

pub struct BaaLSContractEngine<S: Storage> {
    _storage_marker: std::marker::PhantomData<S>,
    wasm_engine: Engine,
    resource_limits: ResourceLimits,
    contract_metrics: Arc<Mutex<HashMap<ContractId, ContractMetrics>>>,
    call_depth: Arc<AtomicU32>,
    executing_contracts: Arc<Mutex<HashMap<ContractId, u32>>>,
    inter_contract_results: Arc<Mutex<HashMap<ContractId, Vec<Vec<u8>>>>>,
}

type WasmResult = (Vec<u8>, u64, Vec<(Vec<u8>, Vec<u8>)>);

impl<S: Storage> BaaLSContractEngine<S> {
    pub fn new(_storage: S) -> Result<Self, ContractError> {
        let limits = ResourceLimits::default();
        let mut config = Config::default();
        config.consume_fuel(true);
        config.max_wasm_stack(limits.stack_limit as usize);
        config.static_memory_maximum_size(limits.memory_limit as u64);
        let wasm_engine =
            Engine::new(&config).map_err(|e| ContractError::WasmRuntimeError(e.to_string()))?;
        Ok(Self {
            _storage_marker: std::marker::PhantomData,
            wasm_engine,
            resource_limits: limits,
            contract_metrics: Arc::new(Mutex::new(HashMap::new())),
            call_depth: Arc::new(AtomicU32::new(0)),
            executing_contracts: Arc::new(Mutex::new(HashMap::new())),
            inter_contract_results: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn update_contract_metrics(
        &self,
        contract_id: &ContractId,
        execution_result: &ContractExecutionResult,
    ) {
        let mut metrics = self.contract_metrics.lock().unwrap();
        let cm = metrics.entry(contract_id.clone()).or_insert_with(|| ContractMetrics {
            total_calls: 0,
            total_gas_used: 0,
            average_execution_time: Duration::ZERO,
            success_rate: 1.0,
            last_called: None,
            memory_usage_stats: MemoryUsageStats {
                peak_memory: 0,
                average_memory: 0,
                memory_growth_rate: 0.0,
            },
        });

        cm.total_calls += 1;
        cm.total_gas_used += execution_result.gas_used;
        cm.last_called = Some(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        );

        let total_time = cm.average_execution_time * (cm.total_calls.saturating_sub(1)) as u32;
        cm.average_execution_time =
            (total_time + execution_result.execution_time) / cm.total_calls as u32;

        if execution_result.success {
            cm.success_rate =
                (cm.success_rate * (cm.total_calls - 1) as f64 + 1.0) / cm.total_calls as f64;
        } else {
            cm.success_rate =
                (cm.success_rate * (cm.total_calls - 1) as f64) / cm.total_calls as f64;
        }

        if execution_result.memory_used > cm.memory_usage_stats.peak_memory {
            cm.memory_usage_stats.peak_memory = execution_result.memory_used;
        }
    }

    // ─── WASM Execution ───

    #[allow(clippy::too_many_arguments)]
    pub fn execute_wasm_contract(
        &self,
        wasm_bytes: &[u8],
        method_name: &str,
        args: &[Vec<u8>],
        caller: &PublicKey,
        contract_id: &ContractId,
        storage: &dyn Storage,
        read_only: bool,
        gas_limit: u64,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<WasmResult, ContractError> {
        let module = Module::new(&self.wasm_engine, wasm_bytes)
            .map_err(|e| ContractError::InvalidWasm(e.to_string()))?;

        let engine = module.engine();

        // Serialize multi-args to flat bytes for WASM
        let serialized_args = bincode::serialize(args).map_err(|e| {
            ContractError::ExecutionError(format!("Args serialization failed: {}", e))
        })?;

        // Build host state
        let pre_existing_results = {
            let engine_results = self.inter_contract_results.lock().unwrap();
            engine_results.get(contract_id).cloned().unwrap_or_default()
        };
        let host_state = HostState {
            caller: *caller,
            contract_id: contract_id.clone(),
            contract_storage: HashMap::new(),
            storage: storage.clone_storage(),
            read_only,
            gas_used: 0,
            gas_limit,
            block_index,
            block_timestamp,
            input_data: serialized_args.clone(),
            reverted: false,
            events: Vec::new(),
            last_memory_size: 0,
            inter_contract_calls: Vec::new(),
            inter_contract_results: pre_existing_results,
            deleted_keys: Vec::new(),
            permissions: ContractPermissions::all(),
        };

        let mut store = Store::new(engine, host_state);
        // Set fuel for gas metering
        store.set_fuel(gas_limit).map_err(|e| ContractError::WasmRuntimeError(e.to_string()))?;

        let mut linker = Linker::new(engine);
        self.add_host_functions(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| ContractError::ExecutionError(format!("Instantiation failed: {}", e)))?;

        // Try the method name directly, then with leading underscore, then "main"
        let func = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, method_name)
            .or_else(|_| {
                let alt = format!("_{}", method_name);
                instance.get_typed_func::<(i32, i32), i32>(&mut store, &alt)
            })
            .or_else(|_| instance.get_typed_func::<(i32, i32), i32>(&mut store, "main"))
            .map_err(|_| {
                ContractError::ExecutionError(format!(
                    "Export '{}' not found in WASM module",
                    method_name
                ))
            })?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| ContractError::ExecutionError("Memory export not found".to_string()))?;

        // Write serialized args to WASM memory
        let args_len = serialized_args.len() as i32;
        if args_len > 0 {
            memory
                .write(&mut store, 0, &serialized_args)
                .map_err(|e| ContractError::MemoryAccessError(e.to_string()))?;
        }

        let start_time = Instant::now();

        let result = func.call(&mut store, (0i32, args_len));

        let execution_time = start_time.elapsed();
        let remaining_fuel = store.get_fuel().unwrap_or(0);
        let gas_used = gas_limit.saturating_sub(remaining_fuel);

        match result {
            Ok(ret_val) => {
                // Read result from memory before taking any mutable store borrows
                let result_data = if ret_val > 0 {
                    let mut buffer = vec![0u8; ret_val as usize];
                    memory
                        .read(&store, 0, &mut buffer)
                        .map_err(|e| ContractError::MemoryAccessError(e.to_string()))?;
                    buffer
                } else {
                    vec![]
                };

                let host_state = store.data_mut();
                if host_state.reverted {
                    return Err(ContractError::Reverted(
                        "Contract reverted during execution".to_string(),
                    ));
                }

                // Commit storage changes
                if !read_only {
                    for (key, value) in &host_state.contract_storage {
                        host_state
                            .storage
                            .contract_storage_write(&host_state.contract_id, key, value)
                            .map_err(ContractError::StorageError)?;
                    }
                    // Commit deletions
                    for key in &host_state.deleted_keys {
                        host_state
                            .storage
                            .contract_storage_remove(&host_state.contract_id, key)
                            .ok();
                    }
                    // Persist emitted events
                    for (topic, data) in &host_state.events {
                        host_state
                            .storage
                            .contract_emit_event(&host_state.contract_id, topic, data)
                            .ok();
                    }
                }

                let events = host_state.events.clone();
                let pending_calls = std::mem::take(&mut host_state.inter_contract_calls);

                // Capture storage reference (avoid borrow conflicts)
                let storage_ref = storage.clone_storage();

                info!(
                    "[CONTRACTS] {}::{} executed: gas={}, time={:?}",
                    hex::encode(contract_id.to_bytes()),
                    method_name,
                    gas_used,
                    execution_time
                );

                // Process inter-contract calls after initial execution
                let mut call_results = Vec::new();
                for (callee_id_bytes, method_bytes, call_args, _call_value) in &pending_calls {
                    let callee_method = String::from_utf8_lossy(method_bytes).to_string();
                    // Look up callee contract code
                    let callee_id = if callee_id_bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(callee_id_bytes);
                        ContractId::from_bytes(&arr)
                    } else {
                        warn!("[CONTRACTS] Invalid callee ID length in inter-contract call");
                        call_results.push(vec![]);
                        continue;
                    };

                    let callee_wasm = match storage_ref.get_contract_code(&callee_id) {
                        Ok(Some(code)) => code,
                        _ => {
                            warn!(
                                "[CONTRACTS] Callee contract {} not found",
                                hex::encode(callee_id.to_bytes())
                            );
                            call_results.push(vec![]);
                            continue;
                        }
                    };

                    // Reentrancy guard for inter-contract calls
                    {
                        let mut executing = self.executing_contracts.lock().unwrap();
                        let entry = executing.entry(callee_id.clone()).or_insert(0);
                        *entry += 1;
                        if *entry > 1 {
                            warn!(
                                "[CONTRACTS] Reentrancy blocked on inter-contract call to {}",
                                hex::encode(callee_id.to_bytes())
                            );
                            call_results.push(vec![]);
                            continue;
                        }
                    }
                    let _inter_guard = ReentrancyGuard {
                        executing_contracts: self.executing_contracts.clone(),
                        contract_id: callee_id.clone(),
                    };

                    // Execute callee inline (synchronous inter-contract call)
                    match self.execute_wasm_contract(
                        &callee_wasm,
                        &callee_method,
                        call_args,
                        caller,
                        &callee_id,
                        &*storage_ref,
                        read_only,
                        gas_limit.saturating_sub(gas_used),
                        block_index,
                        block_timestamp,
                    ) {
                        Ok((result_data, _, _)) => {
                            call_results.push(result_data);
                        }
                        Err(e) => {
                            warn!(
                                "[CONTRACTS] Inter-contract call to {}::{} failed: {}",
                                hex::encode(callee_id.to_bytes()),
                                callee_method,
                                e
                            );
                            call_results.push(vec![]);
                        }
                    }
                }
                // Store results for baals_read_call_result in case the initial caller reads them
                store.data_mut().inter_contract_results = call_results.clone();
                // Persist results in engine for subsequent contract calls within this block
                {
                    let mut engine_results = self.inter_contract_results.lock().unwrap();
                    engine_results.insert(contract_id.clone(), call_results);
                }

                Ok((result_data, gas_used, events))
            }
            Err(e) => {
                let host_state = store.data();
                let _gas_used = host_state.gas_used;
                warn!(
                    "[CONTRACTS] {}::{} trap: {}",
                    hex::encode(contract_id.to_bytes()),
                    method_name,
                    e
                );
                Err(ContractError::ExecutionError(e.to_string()))
            }
        }
    }

    // ─── Host Functions ───

    fn add_host_functions(&self, linker: &mut Linker<HostState>) -> Result<(), ContractError> {
        fn checked_len(len: i32, max: usize) -> Option<usize> {
            if len < 0 || len as usize > max {
                None
            } else {
                Some(len as usize)
            }
        }

        // baals_storage_read(key_ptr, key_len, value_ptr, value_len_cap) -> bytes_written
        linker
            .func_wrap(
                "env",
                "baals_storage_read",
                |mut caller: Caller<'_, HostState>,
                 key_ptr: i32,
                 key_len: i32,
                 value_ptr: i32,
                 value_len_cap: i32|
                 -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let key_len = match checked_len(key_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    if value_len_cap < 0 {
                        return -1;
                    }
                    let mut key = vec![0u8; key_len];
                    if mem.read(&caller, key_ptr as usize, &mut key).is_err() {
                        return -1;
                    }

                    let value = {
                        let state = caller.data_mut();
                        state.charge_gas(100).ok();
                        state.contract_storage.get(&key).cloned().or_else(|| {
                            state
                                .storage
                                .contract_storage_read(&state.contract_id, &key)
                                .ok()
                                .flatten()
                        })
                    };

                    match value {
                        Some(v) => {
                            let len = v.len().min(value_len_cap as usize);
                            if mem.write(&mut caller, value_ptr as usize, &v[..len]).is_err() {
                                return -1;
                            }
                            len as i32
                        }
                        None => 0,
                    }
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_storage_write(key_ptr, key_len, value_ptr, value_len)
        linker
            .func_wrap(
                "env",
                "baals_storage_write",
                |mut caller: Caller<'_, HostState>,
                 key_ptr: i32,
                 key_len: i32,
                 value_ptr: i32,
                 value_len: i32|
                 -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let key_len = match checked_len(key_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let value_len = match checked_len(value_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let mut key = vec![0u8; key_len];
                    let mut value = vec![0u8; value_len];
                    if mem.read(&caller, key_ptr as usize, &mut key).is_err() {
                        return -1;
                    }
                    if mem.read(&caller, value_ptr as usize, &mut value).is_err() {
                        return -1;
                    }

                    let state = caller.data_mut();
                    state.charge_gas(200).ok();
                    if state.read_only {
                        return -1;
                    }
                    if !state.permissions.storage_write {
                        return -1;
                    }
                    state.contract_storage.insert(key, value);
                    0
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_storage_remove(key_ptr, key_len)
        linker
            .func_wrap(
                "env",
                "baals_storage_remove",
                |mut caller: Caller<'_, HostState>, key_ptr: i32, key_len: i32| -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let key_len = match checked_len(key_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let mut key = vec![0u8; key_len];
                    if mem.read(&caller, key_ptr as usize, &mut key).is_err() {
                        return -1;
                    }

                    let state = caller.data_mut();
                    state.charge_gas(50).ok();
                    if state.read_only {
                        return -1;
                    }
                    // Actually remove the key, not just insert empty vec
                    state.contract_storage.remove(&key);
                    state.deleted_keys.push(key);
                    0
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_get_sender(ptr) — writes 32-byte sender public key
        linker
            .func_wrap("env", "baals_get_sender", |mut caller: Caller<'_, HostState>, ptr: i32| {
                let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();
                let sender_bytes = caller.data().caller.to_bytes();
                let _ = mem.write(&mut caller, ptr as usize, &sender_bytes);
            })
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_get_contract_id(ptr) — writes 32-byte contract ID
        linker
            .func_wrap(
                "env",
                "baals_get_contract_id",
                |mut caller: Caller<'_, HostState>, ptr: i32| {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();
                    let cid_bytes = caller.data().contract_id.to_bytes();
                    let _ = mem.write(&mut caller, ptr as usize, &cid_bytes);
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_get_block_timestamp() -> u64
        linker
            .func_wrap("env", "baals_get_block_timestamp", |caller: Caller<'_, HostState>| -> u64 {
                caller.data().block_timestamp
            })
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_get_block_index() -> u64
        linker
            .func_wrap("env", "baals_get_block_index", |caller: Caller<'_, HostState>| -> u64 {
                caller.data().block_index
            })
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_get_input_data(ptr, len_cap) -> bytes_written
        linker
            .func_wrap(
                "env",
                "baals_get_input_data",
                |mut caller: Caller<'_, HostState>, ptr: i32, len_cap: i32| -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();
                    let data = caller.data().input_data.clone();
                    let len = data.len().min(len_cap as usize);
                    let _ = mem.write(&mut caller, ptr as usize, &data[..len]);
                    len as i32
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_hash_sha256(data_ptr, data_len, output_ptr)
        linker
            .func_wrap(
                "env",
                "baals_hash_sha256",
                |mut caller: Caller<'_, HostState>,
                 data_ptr: i32,
                 data_len: i32,
                 output_ptr: i32| {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let data_len = match checked_len(data_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return,
                    };
                    let mut data = vec![0u8; data_len];
                    if mem.read(&caller, data_ptr as usize, &mut data).is_err() {
                        return;
                    }

                    caller.data_mut().charge_gas(50 + data_len as u64 / 16).ok();

                    let mut hasher = Sha256::new();
                    hasher.update(&data);
                    let hash = hasher.finalize();
                    let _ = mem.write(&mut caller, output_ptr as usize, &hash);
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_verify_signature(pubkey_ptr, pubkey_len, msg_ptr, msg_len, sig_ptr, sig_len) -> 0=invalid, 1=valid
        linker
            .func_wrap(
                "env",
                "baals_verify_signature",
                |mut caller: Caller<'_, HostState>,
                 pubkey_ptr: i32,
                 pubkey_len: i32,
                 msg_ptr: i32,
                 msg_len: i32,
                 sig_ptr: i32,
                 sig_len: i32|
                 -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let pubkey_len = match checked_len(pubkey_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let msg_len = match checked_len(msg_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let sig_len = match checked_len(sig_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };

                    let mut pk_bytes = vec![0u8; pubkey_len];
                    let mut msg = vec![0u8; msg_len];
                    let mut sig_bytes = vec![0u8; sig_len];

                    if mem.read(&caller, pubkey_ptr as usize, &mut pk_bytes).is_err() {
                        return 0;
                    }
                    if mem.read(&caller, msg_ptr as usize, &mut msg).is_err() {
                        return 0;
                    }
                    if mem.read(&caller, sig_ptr as usize, &mut sig_bytes).is_err() {
                        return 0;
                    }

                    caller.data_mut().charge_gas(500).ok();

                    if pk_bytes.len() != 32 || sig_bytes.len() != 64 {
                        return 0;
                    }

                    let pk_arr: [u8; 32] = match pk_bytes.try_into() {
                        Ok(a) => a,
                        Err(_) => return 0,
                    };
                    let sig_arr: [u8; 64] = match sig_bytes.try_into() {
                        Ok(a) => a,
                        Err(_) => return 0,
                    };

                    let pk = match PublicKey::from_bytes(&pk_arr) {
                        Ok(p) => p,
                        Err(_) => return 0,
                    };
                    let sig = match TransactionSignature::from_bytes(&sig_arr) {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    let verify_sig = Ed25519Signature::from(sig);
                    if pk.verify(&msg, &verify_sig).is_ok() {
                        1
                    } else {
                        0
                    }
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_emit_event(topic_ptr, topic_len, data_ptr, data_len)
        linker
            .func_wrap(
                "env",
                "baals_emit_event",
                |mut caller: Caller<'_, HostState>,
                 topic_ptr: i32,
                 topic_len: i32,
                 data_ptr: i32,
                 data_len: i32| {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let topic_len = match checked_len(topic_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return,
                    };
                    let data_len = match checked_len(data_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return,
                    };
                    let mut topic = vec![0u8; topic_len];
                    let mut data = vec![0u8; data_len];
                    let _ = mem.read(&caller, topic_ptr as usize, &mut topic);
                    let _ = mem.read(&caller, data_ptr as usize, &mut data);

                    let state = caller.data_mut();
                    if !state.permissions.emit_events {
                        return;
                    }
                    state.charge_gas(100 + topic_len as u64 + data_len as u64).ok();
                    state.events.push((topic, data));
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_revert(msg_ptr, msg_len) — sets the reverted flag; execution continues but caller gets error
        linker
            .func_wrap(
                "env",
                "baals_revert",
                |mut caller: Caller<'_, HostState>, _msg_ptr: i32, msg_len: i32| {
                    if checked_len(msg_len, 1024 * 1024).is_none() {
                        return;
                    }
                    let state = caller.data_mut();
                    state.reverted = true;
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_memory_grow — tracks memory growth and charges gas
        linker
            .func_wrap("env", "baals_memory_grow", |mut caller: Caller<'_, HostState>| -> i32 {
                let mem = match caller.get_export("memory") {
                    Some(e) => match e.into_memory() {
                        Some(m) => m,
                        None => return -1,
                    },
                    None => return -1,
                };
                let current_size = mem.data_size(&caller);
                let state = caller.data_mut();
                if state.charge_memory_growth(current_size).is_err() {
                    return -1;
                }
                current_size as i32
            })
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_call_contract(callee_ptr, callee_len, method_ptr, method_len, args_ptr, args_len, value: i64) -> i32
        linker
            .func_wrap(
                "env",
                "baals_call_contract",
                |mut caller: Caller<'_, HostState>,
                 callee_ptr: i32,
                 callee_len: i32,
                 method_ptr: i32,
                 method_len: i32,
                 args_ptr: i32,
                 args_len: i32,
                 value: i64|
                 -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();

                    let callee_len = match checked_len(callee_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let method_len = match checked_len(method_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };
                    let args_len = match checked_len(args_len, 1024 * 1024) {
                        Some(n) => n,
                        None => return -1,
                    };

                    let mut callee = vec![0u8; callee_len];
                    let mut method = vec![0u8; method_len];
                    let mut args = vec![0u8; args_len];
                    if mem.read(&caller, callee_ptr as usize, &mut callee).is_err()
                        || mem.read(&caller, method_ptr as usize, &mut method).is_err()
                        || mem.read(&caller, args_ptr as usize, &mut args).is_err()
                    {
                        return -1;
                    }
                    let state = caller.data_mut();
                    state.charge_gas(1000).ok();
                    if state.read_only {
                        return -1;
                    }
                    if !state.permissions.call_contracts {
                        return -1;
                    }
                    let native_value = if value < 0 { 0 } else { value as u64 };
                    let parsed_args: Vec<Vec<u8>> =
                        bincode::deserialize(&args).unwrap_or_else(|_| vec![args.to_vec()]);
                    state.inter_contract_calls.push((callee, method, parsed_args, native_value));
                    state.inter_contract_calls.len() as i32 - 1
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        // baals_read_call_result(result_ptr, result_len_cap, call_index) -> i32
        linker
            .func_wrap(
                "env",
                "baals_read_call_result",
                |mut caller: Caller<'_, HostState>,
                 result_ptr: i32,
                 result_len_cap: i32,
                 call_index: i32|
                 -> i32 {
                    let mem = caller.get_export("memory").and_then(|e| e.into_memory()).unwrap();
                    let state = caller.data();
                    let idx = call_index as usize;
                    if idx >= state.inter_contract_results.len() {
                        return -1;
                    }
                    let result = state.inter_contract_results[idx].clone();
                    let _ = state;
                    let len = result.len().min(result_len_cap as usize);
                    let _ = mem.write(&mut caller, result_ptr as usize, &result[..len]);
                    len as i32
                },
            )
            .map_err(|e| ContractError::HostFunctionError(e.to_string()))?;

        Ok(())
    }
    fn validate_wasm_module(module: &wasmtime::Module) -> Result<(), ContractError> {
        // Verify memory export exists
        let has_memory = module
            .exports()
            .any(|e| e.name() == "memory" && matches!(e.ty(), wasmtime::ExternType::Memory(_)));
        if !has_memory {
            return Err(ContractError::BytecodeValidationFailed(
                "WASM module must export a 'memory'".to_string(),
            ));
        }

        // Validate imports are only from allowed modules
        let allowed_modules = ["env"];
        for import in module.imports() {
            let mod_name = import.module();
            if !allowed_modules.contains(&mod_name) {
                return Err(ContractError::BytecodeValidationFailed(format!(
                    "Disallowed import module: '{}'",
                    mod_name
                )));
            }
        }

        // Check memory limits from the module
        if let Some(wasmtime::ExternType::Memory(mem)) = module
            .imports()
            .find(|i| matches!(i.ty(), wasmtime::ExternType::Memory(_)))
            .map(|i| i.ty())
            .or_else(|| {
                module
                    .exports()
                    .find(|e| matches!(e.ty(), wasmtime::ExternType::Memory(_)))
                    .map(|e| e.ty())
            })
        {
            if mem.minimum() > 1024 {
                return Err(ContractError::BytecodeValidationFailed(format!(
                    "Memory minimum pages ({}) exceeds limit (1024)",
                    mem.minimum()
                )));
            }
            if let Some(max) = mem.maximum() {
                if max > 4096 {
                    return Err(ContractError::BytecodeValidationFailed(format!(
                        "Memory maximum pages ({}) exceeds limit (4096)",
                        max
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn scan_for_float_opcodes(wasm_bytes: &[u8]) -> Result<(), String> {
        let mut i = 8; // skip WASM magic + version
        loop {
            if i >= wasm_bytes.len() {
                break;
            }
            let section_id = wasm_bytes[i];
            i += 1;
            if i >= wasm_bytes.len() {
                break;
            }
            // Read section length (LEB128 u32)
            let mut section_len = 0usize;
            let mut shift = 0;
            loop {
                if i >= wasm_bytes.len() {
                    return Err("Unexpected end of WASM data".to_string());
                }
                let byte = wasm_bytes[i] as usize;
                section_len |= (byte & 0x7F) << shift;
                i += 1;
                shift += 7;
                if byte & 0x80 == 0 {
                    break;
                }
                if shift > 35 {
                    return Err("Section length overflow".to_string());
                }
            }
            if section_id == 0x0A {
                // Code section: scan function bodies for float opcodes
                let section_end = (i + section_len).min(wasm_bytes.len());
                while i < section_end {
                    let op = wasm_bytes[i];
                    match op {
                        0x2A..=0x2D => {
                            return Err(format!(
                                "Non-deterministic float memory opcode 0x{:02X} at byte {}",
                                op, i
                            ))
                        }
                        0x43..=0x44 => {
                            return Err(format!("Float constant opcode 0x{:02X} at byte {}", op, i))
                        }
                        0x5D..=0x66 => {
                            return Err(format!(
                                "Float comparison opcode 0x{:02X} at byte {}",
                                op, i
                            ))
                        }
                        0x8E..=0xA2 => {
                            return Err(format!(
                                "Float arithmetic opcode 0x{:02X} at byte {}",
                                op, i
                            ))
                        }
                        _ => {}
                    }
                    i += 1;
                }
            } else {
                i += section_len;
            }
        }
        Ok(())
    }
}

// ─── WasmRuntime trait impl ───

impl<S: Storage> WasmRuntime for BaaLSContractEngine<S> {
    fn execute_wasm_contract(
        &self,
        wasm_bytes: &[u8],
        method_name: &str,
        args: &[Vec<u8>],
        caller: &PublicKey,
        contract_id: &ContractId,
        storage: &dyn Storage,
        read_only: bool,
        gas_limit: u64,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<(Vec<u8>, u64, Vec<(Vec<u8>, Vec<u8>)>), ContractError> {
        BaaLSContractEngine::execute_wasm_contract(
            self,
            wasm_bytes,
            method_name,
            args,
            caller,
            contract_id,
            storage,
            read_only,
            gas_limit,
            block_index,
            block_timestamp,
        )
    }

    fn validate_wasm_module(&self, wasm_bytes: &[u8]) -> Result<(), ContractError> {
        let module = wasmtime::Module::new(&self.wasm_engine, wasm_bytes)
            .map_err(|e| ContractError::InvalidWasm(e.to_string()))?;
        Self::validate_wasm_module(&module)
    }

    fn estimate_gas(
        &self,
        wasm_bytes: &[u8],
        _method_name: &str,
        _args: &[Vec<u8>],
    ) -> Result<u64, ContractError> {
        // Base cost + bytecode size proportional cost
        Ok(21_000 + (wasm_bytes.len() as u64 / 100))
    }
}

// ─── ContractEngine trait impl ───

impl<S: Storage + 'static> ContractEngine for BaaLSContractEngine<S> {
    fn deploy_contract(
        &self,
        deployer: &PublicKey,
        deployer_nonce: u64,
        wasm_bytes: &[u8],
        init_payload: Option<&[u8]>,
        storage: &dyn Storage,
        gas_limit: u64,
    ) -> Result<ContractId, ContractError> {
        // Validate WASM bytecode
        if wasm_bytes.is_empty() {
            return Err(ContractError::BytecodeValidationFailed(
                "WASM bytecode is empty".to_string(),
            ));
        }
        if wasm_bytes.len() > 10 * 1024 * 1024 {
            return Err(ContractError::BytecodeValidationFailed(
                "WASM bytecode exceeds 10MB limit".to_string(),
            ));
        }
        if &wasm_bytes[0..4] != b"\x00asm" {
            return Err(ContractError::BytecodeValidationFailed(
                "Invalid WASM magic bytes".to_string(),
            ));
        }
        // Ban non-deterministic float opcodes
        Self::scan_for_float_opcodes(wasm_bytes)
            .map_err(ContractError::BytecodeValidationFailed)?;
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| ContractError::InvalidWasm(e.to_string()))?;

        // Deep WASM validation
        Self::validate_wasm_module(&module)?;

        // Validate that contract exports at least one function
        let has_exports = module.exports().any(|e| matches!(e.ty(), wasmtime::ExternType::Func(_)));
        if !has_exports {
            return Err(ContractError::BytecodeValidationFailed(
                "Contract must export at least one function".to_string(),
            ));
        }

        // Generate contract ID from deployer + deployer_nonce + WASM hash + init_payload
        let mut hasher = Sha256::new();
        hasher.update(deployer.to_bytes());
        hasher.update(deployer_nonce.to_be_bytes());
        hasher.update(wasm_bytes);
        if let Some(payload) = init_payload {
            hasher.update(payload);
        }
        let hash = hasher.finalize();
        let contract_id = ContractId::from_bytes(&hash.into());

        // Store contract code
        storage.put_contract_code(&contract_id, wasm_bytes).map_err(ContractError::StorageError)?;

        // Store deployer address
        storage
            .put_contract_deployer(&contract_id, deployer)
            .map_err(ContractError::StorageError)?;

        // If init_payload is provided, call the contract's init/instantiate function
        if let Some(payload) = init_payload {
            if !payload.is_empty() {
                info!(
                    "[CONTRACTS] Running init for contract {} with payload len={}",
                    hex::encode(contract_id.to_bytes()),
                    payload.len()
                );
                let init_args = [payload.to_vec()];
                match self.execute_wasm_contract(
                    wasm_bytes,
                    "init",
                    &init_args,
                    deployer,
                    &contract_id,
                    storage,
                    false,
                    gas_limit,
                    0,
                    0,
                ) {
                    Ok(_) => info!(
                        "[CONTRACTS] Init successful for {}",
                        hex::encode(contract_id.to_bytes())
                    ),
                    Err(e) => warn!(
                        "[CONTRACTS] Init failed for {}: {}",
                        hex::encode(contract_id.to_bytes()),
                        e
                    ),
                }
            }
        }

        info!("[CONTRACTS] Contract deployed: {}", hex::encode(contract_id.to_bytes()));
        Ok(contract_id)
    }

    fn call_contract(
        &self,
        caller: &PublicKey,
        contract_id: &ContractId,
        method_name: &str,
        args: &[Vec<u8>],
        value: Option<u64>,
        storage: &dyn Storage,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<Vec<u8>, ContractError> {
        // Reentrancy guard: check if this contract is already executing
        {
            let mut executing = self.executing_contracts.lock().unwrap();
            let entry = executing.entry(contract_id.clone()).or_insert(0);
            *entry += 1;
            if *entry > 1 {
                return Err(ContractError::ReentrancyDetected(hex::encode(contract_id.to_bytes())));
            }
        }
        // Cleanup on scope exit
        let _guard = ReentrancyGuard {
            executing_contracts: self.executing_contracts.clone(),
            contract_id: contract_id.clone(),
        };

        // Track call depth
        let depth = self.call_depth.fetch_add(1, Ordering::SeqCst);
        if depth > 64 {
            self.call_depth.fetch_sub(1, Ordering::SeqCst);
            return Err(ContractError::ResourceLimitExceeded(
                "Max call depth exceeded (64)".to_string(),
            ));
        }
        let _depth_guard = CallDepthGuard { call_depth: self.call_depth.clone() };

        let wasm_bytes = storage
            .get_contract_code(contract_id)
            .map_err(ContractError::StorageError)?
            .ok_or_else(|| {
                ContractError::ContractNotFound(format!(
                    "Contract not found: {}",
                    hex::encode(contract_id.to_bytes())
                ))
            })?;

        // Check deployer — log warning if caller is not the deployer
        if let Ok(Some(deployer)) = storage.get_contract_deployer(contract_id) {
            if deployer != *caller {
                warn!(
                    "[CONTRACTS] Caller {} is not the deployer of contract {}",
                    hex::encode(caller.to_bytes()),
                    hex::encode(contract_id.to_bytes())
                );
            }
        }

        // Validate that the requested method exists as a module export
        {
            let module = wasmtime::Module::new(&self.wasm_engine, &wasm_bytes)
                .map_err(|e| ContractError::InvalidWasm(e.to_string()))?;
            let has_export = module.exports().any(|e| {
                e.name() == method_name
                    || e.name() == format!("_{}", method_name)
                    || e.name() == "main"
            });
            if !has_export {
                return Err(ContractError::ExecutionError(format!(
                    "Method '{}' not found in contract exports",
                    method_name
                )));
            }
        }

        let _ = value; // value transfer handled by ledger

        let (result, gas_used, events) = self.execute_wasm_contract(
            &wasm_bytes,
            method_name,
            args,
            caller,
            contract_id,
            storage,
            false,
            self.resource_limits.gas_limit,
            block_index,
            block_timestamp,
        )?;

        let execution_result = ContractExecutionResult {
            success: true,
            output_data: Some(result.clone()),
            gas_used,
            error_message: None,
            execution_time: Duration::ZERO,
            memory_used: 0,
            instructions_executed: gas_used,
        };
        self.update_contract_metrics(contract_id, &execution_result);

        if !events.is_empty() {
            info!(
                "[CONTRACTS] {} events emitted by {}::{}",
                events.len(),
                hex::encode(contract_id.to_bytes()),
                method_name
            );
        }

        Ok(result)
    }

    fn query_contract(
        &self,
        contract_id: &ContractId,
        method_name: &str,
        payload: &[u8],
        storage: &dyn Storage,
        block_index: u64,
        block_timestamp: u64,
    ) -> Result<Vec<u8>, ContractError> {
        let wasm_bytes = storage
            .get_contract_code(contract_id)
            .map_err(ContractError::StorageError)?
            .ok_or_else(|| {
                ContractError::ContractNotFound(format!(
                    "Contract not found: {}",
                    hex::encode(contract_id.to_bytes())
                ))
            })?;

        let dummy_caller = PublicKey::from_bytes(&[0; 32])
            .map_err(|_| ContractError::ExecutionError("Invalid dummy key".to_string()))?;

        if let Ok(Some(deployer)) = storage.get_contract_deployer(contract_id) {
            if deployer != dummy_caller {
                warn!(
                    "[CONTRACTS] Query caller {} is not the deployer of contract {}",
                    hex::encode(dummy_caller.to_bytes()),
                    hex::encode(contract_id.to_bytes())
                );
            }
        }

        let args = [payload.to_vec()];
        let (result, _, _) = self.execute_wasm_contract(
            &wasm_bytes,
            method_name,
            &args,
            &dummy_caller,
            contract_id,
            storage,
            true,
            self.resource_limits.gas_limit,
            block_index,
            block_timestamp,
        )?;
        Ok(result)
    }

    fn verify_contract(
        &self,
        wasm_bytes: &[u8],
        _contract_id: &ContractId,
    ) -> Result<(), ContractError> {
        let engine = Engine::default();
        Module::new(&engine, wasm_bytes).map_err(|e| ContractError::InvalidWasm(e.to_string()))?;
        Ok(())
    }

    fn estimate_gas_usage(
        &self,
        contract_id: &ContractId,
        method_name: &str,
        args: &[Vec<u8>],
        storage: &dyn Storage,
    ) -> Result<GasEstimate, ContractError> {
        let wasm_bytes = match storage.get_contract_code(contract_id) {
            Ok(Some(bytes)) => bytes,
            _ => {
                return Ok(GasEstimate {
                    estimated_gas: 21000,
                    confidence_level: 0.1,
                    execution_time_estimate: Duration::from_millis(1),
                });
            }
        };

        let dummy_caller = PublicKey::from_bytes(&[0; 32])
            .map_err(|_| ContractError::ExecutionError("Invalid dummy key".to_string()))?;

        // Dry-run execution to measure actual gas used
        let start = Instant::now();
        let estimate_gas = 1_000_000u64; // high gas limit for estimation
        match self.execute_wasm_contract(
            &wasm_bytes,
            method_name,
            args,
            &dummy_caller,
            contract_id,
            storage,
            true, // read-only
            estimate_gas,
            0,
            0,
        ) {
            Ok((_, gas_used, _)) => {
                let elapsed = start.elapsed();
                // Add 20% buffer for safety
                let estimated = ((gas_used as f64) * 1.2).ceil() as u64;
                Ok(GasEstimate {
                    estimated_gas: estimated.max(21000),
                    confidence_level: 0.9,
                    execution_time_estimate: elapsed,
                })
            }
            Err(_e) => {
                // If execution fails, fall back to a reasonable default
                Ok(GasEstimate {
                    estimated_gas: 21000,
                    confidence_level: 0.1,
                    execution_time_estimate: Duration::from_millis(5),
                })
            }
        }
    }

    fn get_contract_metrics(
        &self,
        contract_id: &ContractId,
    ) -> Result<ContractMetrics, ContractError> {
        let metrics = self.contract_metrics.lock().unwrap();
        metrics.get(contract_id).cloned().ok_or_else(|| {
            ContractError::ContractNotFound(format!(
                "Contract {} not found",
                hex::encode(contract_id.to_bytes())
            ))
        })
    }

    fn set_resource_limits(
        &mut self,
        memory_limit: usize,
        table_limit: u32,
        stack_limit: u32,
    ) -> Result<(), ContractError> {
        self.resource_limits.memory_limit = memory_limit;
        self.resource_limits.table_limit = table_limit;
        self.resource_limits.stack_limit = stack_limit;
        Ok(())
    }
}

// Reentrancy guard: decrements counter on drop
struct ReentrancyGuard {
    executing_contracts: Arc<Mutex<HashMap<ContractId, u32>>>,
    contract_id: ContractId,
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        let mut executing = self.executing_contracts.lock().unwrap();
        if let Some(entry) = executing.get_mut(&self.contract_id) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                executing.remove(&self.contract_id);
            }
        }
    }
}

// Call depth guard: decrements counter on drop
struct CallDepthGuard {
    call_depth: Arc<AtomicU32>,
}

impl Drop for CallDepthGuard {
    fn drop(&mut self) {
        self.call_depth.fetch_sub(1, Ordering::SeqCst);
    }
}
