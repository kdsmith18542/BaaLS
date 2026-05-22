//! Raw FFI bindings to BaaLS host functions.

extern "C" {
    pub fn baals_storage_read(
        key_ptr: i32,
        key_len: i32,
        value_ptr: i32,
        value_len_cap: i32,
    ) -> i32;

    pub fn baals_storage_write(
        key_ptr: i32,
        key_len: i32,
        value_ptr: i32,
        value_len: i32,
    ) -> i32;

    pub fn baals_storage_remove(key_ptr: i32, key_len: i32) -> i32;

    pub fn baals_get_sender(ptr: i32);

    pub fn baals_get_contract_id(ptr: i32);

    pub fn baals_get_block_timestamp() -> u64;

    pub fn baals_get_block_index() -> u64;

    pub fn baals_get_input_data(ptr: i32, len_cap: i32) -> i32;

    pub fn baals_hash_sha256(data_ptr: i32, data_len: i32, output_ptr: i32);

    pub fn baals_verify_signature(
        pubkey_ptr: i32,
        pubkey_len: i32,
        msg_ptr: i32,
        msg_len: i32,
        sig_ptr: i32,
        sig_len: i32,
    ) -> i32;

    pub fn baals_call_contract(
        callee_ptr: i32,
        callee_len: i32,
        method_ptr: i32,
        method_len: i32,
        args_ptr: i32,
        args_len: i32,
        value: i64,
    ) -> i32;

    pub fn baals_read_call_result(
        result_ptr: i32,
        result_len_cap: i32,
        call_index: i32,
    ) -> i32;

    pub fn baals_emit_event(
        topic_ptr: i32,
        topic_len: i32,
        data_ptr: i32,
        data_len: i32,
    );

    pub fn baals_revert(msg_ptr: i32, msg_len: i32);
}

pub fn sender() -> [u8; 32] {
    let mut buf = [0u8; 32];
    unsafe { baals_get_sender(buf.as_mut_ptr() as i32) };
    buf
}

pub fn contract_id() -> [u8; 32] {
    let mut buf = [0u8; 32];
    unsafe { baals_get_contract_id(buf.as_mut_ptr() as i32) };
    buf
}

pub fn block_timestamp() -> u64 {
    unsafe { baals_get_block_timestamp() }
}

pub fn block_index() -> u64 {
    unsafe { baals_get_block_index() }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hash = [0u8; 32];
    unsafe {
        baals_hash_sha256(data.as_ptr() as i32, data.len() as i32, hash.as_mut_ptr() as i32);
    }
    hash
}

pub fn emit_event(topic: &[u8], data: &[u8]) {
    unsafe {
        baals_emit_event(
            topic.as_ptr() as i32,
            topic.len() as i32,
            data.as_ptr() as i32,
            data.len() as i32,
        );
    }
}

pub fn revert(msg: &str) -> ! {
    unsafe {
        baals_revert(msg.as_ptr() as i32, msg.len() as i32);
    }
    unreachable!()
}

pub fn call_contract(callee: &[u8; 32], method: &str, args: &[u8], value: u64) -> i32 {
    unsafe {
        baals_call_contract(
            callee.as_ptr() as i32,
            32,
            method.as_ptr() as i32,
            method.len() as i32,
            args.as_ptr() as i32,
            args.len() as i32,
            value as i64,
        )
    }
}

pub fn read_call_result(call_index: i32, buf: &mut [u8]) -> i32 {
    unsafe {
        baals_read_call_result(buf.as_mut_ptr() as i32, buf.len() as i32, call_index)
    }
}

pub fn get_input_args(len: i32) -> Vec<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    let written = unsafe {
        baals_get_input_data(buf.as_mut_ptr() as i32, len)
    };
    if written < 0 {
        revert("Failed to read input data");
    }
    bincode::deserialize(&buf[..written as usize]).unwrap_or_else(|_| {
        revert("Failed to deserialize input args")
    })
}

pub fn deserialize_arg<T: serde::de::DeserializeOwned>(args: &[Vec<u8>], index: usize) -> T {
    if index >= args.len() {
        revert("Argument index out of bounds");
    }
    bincode::deserialize(&args[index]).unwrap_or_else(|_| {
        revert("Failed to deserialize argument")
    })
}

pub fn return_data<T: serde::Serialize>(value: &T) -> i32 {
    let serialized = bincode::serialize(value).unwrap_or_else(|_| {
        revert("Failed to serialize return data")
    });
    unsafe {
        let dest = std::hint::black_box(0 as *mut u8);
        for (i, &byte) in serialized.iter().enumerate() {
            *dest.add(i) = byte;
        }
    }
    serialized.len() as i32
}

