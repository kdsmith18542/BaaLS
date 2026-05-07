pub fn make_self_calling_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
  (import "env" "baals_get_contract_id" (func $get_id (param i32)))
  (import "env" "baals_call_contract" (func $call_ct (param i32 i32 i32 i32 i32 i32 i64) (result i32)))
  (memory (export "memory") 1)
  (func (export "reenter") (param i32 i32) (result i32)
    (call $get_id (i32.const 0))
    (i32.store8 (i32.const 32) (i32.const 116))
    (i32.store8 (i32.const 33) (i32.const 101))
    (i32.store8 (i32.const 34) (i32.const 115))
    (i32.store8 (i32.const 35) (i32.const 116))
    (i32.store (i32.const 0) (call $call_ct (i32.const 0) (i32.const 32) (i32.const 32) (i32.const 4) (i32.const 36) (i32.const 0) (i64.const 0)))
    i32.const 4
  )
  (func (export "safe") (param i32 i32) (result i32)
    (i32.store (i32.const 0) (i32.const 1))
    i32.const 4
  )
)"#,
    )
    .unwrap()
}

pub fn make_storage_write_read_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
  (import "env" "baals_storage_write" (func $write (param i32 i32 i32 i32) (result i32)))
  (import "env" "baals_storage_read" (func $read (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "key")
  (data (i32.const 16) "val")
  (func (export "store_and_read") (param i32 i32) (result i32)
    (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 3))
    drop
    (call $read (i32.const 0) (i32.const 3) (i32.const 0) (i32.const 200))
  )
)"#,
    )
    .unwrap()
}

pub fn make_inter_contract_callee() -> Vec<u8> {
    wat::parse_str(
        r#"(module
  (import "env" "baals_storage_write" (func $write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "key")
  (data (i32.const 16) "val")
  (func (export "store") (param i32 i32) (result i32)
    (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 3))
    drop
    i32.const 0
  )
)"#,
    )
    .unwrap()
}

/// Create a WASM module that calls another contract at a hardcoded ID.
/// `callee_id` is the 32-byte contract ID to call via `baals_call_contract`
/// with method "main" and no args.
pub fn make_inter_contract_caller(callee_id: &[u8; 32]) -> Vec<u8> {
    let escaped: String = callee_id.iter().map(|b| format!("\\{:02X}", b)).collect();
    let wat_source = format!(
        r#"(module
  (import "env" "baals_call_contract" (func $call_ct (param i32 i32 i32 i32 i32 i32 i64) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{}")
  (data (i32.const 32) "main")
  (func (export "main") (param i32 i32) (result i32)
    (call $call_ct
      (i32.const 0)
      (i32.const 32)
      (i32.const 32)
      (i32.const 4)
      (i32.const 36)
      (i32.const 0)
      (i64.const 0)
    )
    drop
    i32.const 0
  )
)"#,
        escaped
    );
    wat::parse_str(&wat_source).unwrap()
}

pub fn create_test_wasm_module() -> Vec<u8> {
    vec![
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f,
        0x03, 0x02, 0x01, 0x00,
        0x05, 0x03, 0x01, 0x00, 0x01,
        0x07, 0x11, 0x02, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x04, 0x6d, 0x61,
        0x69, 0x6e, 0x00, 0x00,
        0x0a, 0x06, 0x01, 0x04, 0x00, 0x20, 0x01, 0x0b,
    ]
}
