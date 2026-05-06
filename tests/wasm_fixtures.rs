pub fn make_self_calling_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
  (import "env" "baals_get_contract_id" (func $get_id (param i32)))
  (import "env" "baals_call_contract" (func $call_ct (param i32 i32 i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "reenter") (param i32 i32) (result i32)
    (call $get_id (i32.const 0))
    (i32.store8 (i32.const 32) (i32.const 116))
    (i32.store8 (i32.const 33) (i32.const 101))
    (i32.store8 (i32.const 34) (i32.const 115))
    (i32.store8 (i32.const 35) (i32.const 116))
    (i32.store (i32.const 0) (call $call_ct (i32.const 0) (i32.const 32) (i32.const 32) (i32.const 4) (i32.const 36) (i32.const 0)))
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
