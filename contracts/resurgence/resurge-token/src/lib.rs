use resurgence_common::auth::{
    check_role, grant_role, minter_role, pauser_role, revoke_role, DEFAULT_ADMIN_ROLE,
};
use resurgence_common::host::{
    deserialize_arg, emit_event, get_input_args, return_data, revert, sender,
};
use resurgence_common::math::{add, sub};
use resurgence_common::storage::{StorageDoubleMap, StorageMap, StorageValue};
use resurgence_common::types::Address;

const INITIALIZED: StorageValue<bool> = StorageValue::new(b"initialized");
const TOTAL_SUPPLY: StorageValue<u128> = StorageValue::new(b"total_supply");
const CAP: StorageValue<u128> = StorageValue::new(b"cap");
const PAUSED: StorageValue<bool> = StorageValue::new(b"paused");
const BALANCES: StorageMap<Address, u128> = StorageMap::new(b"balances");
const ALLOWANCES: StorageDoubleMap<Address, Address, u128> = StorageDoubleMap::new(b"allowances");

#[no_mangle]
pub extern "C" fn initialize(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let admin: Address = deserialize_arg(&args, 0);
    let token_cap: u128 = deserialize_arg(&args, 1);

    if INITIALIZED.get_or_default() {
        revert("Already initialized");
    }
    INITIALIZED.set(&true);
    CAP.set(&token_cap);

    grant_role(DEFAULT_ADMIN_ROLE, admin);
    grant_role(minter_role(), admin);
    grant_role(pauser_role(), admin);

    return_data(&true)
}

#[no_mangle]
pub extern "C" fn transfer(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let to: Address = deserialize_arg(&args, 0);
    let amount: u128 = deserialize_arg(&args, 1);

    if PAUSED.get_or_default() {
        revert("Pausable: token transfer while paused");
    }

    let owner = sender();
    let balance_owner = BALANCES.get_or_default(&owner);
    if balance_owner < amount {
        revert("ERC20: transfer amount exceeds balance");
    }

    BALANCES.set(&owner, &sub(balance_owner, amount));
    BALANCES.set(&to, &add(BALANCES.get_or_default(&to), amount));

    emit_event(b"Transfer", &bincode::serialize(&(owner, to, amount)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn transfer_from(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let from: Address = deserialize_arg(&args, 0);
    let to: Address = deserialize_arg(&args, 1);
    let amount: u128 = deserialize_arg(&args, 2);

    if PAUSED.get_or_default() {
        revert("Pausable: token transfer while paused");
    }

    let spender = sender();
    let allowance = ALLOWANCES.get_or_default(&from, &spender);
    if allowance < amount {
        revert("ERC20: transfer amount exceeds allowance");
    }

    let balance_from = BALANCES.get_or_default(&from);
    if balance_from < amount {
        revert("ERC20: transfer amount exceeds balance");
    }

    ALLOWANCES.set(&from, &spender, &sub(allowance, amount));
    BALANCES.set(&from, &sub(balance_from, amount));
    BALANCES.set(&to, &add(BALANCES.get_or_default(&to), amount));

    emit_event(b"Transfer", &bincode::serialize(&(from, to, amount)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn approve(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let spender: Address = deserialize_arg(&args, 0);
    let amount: u128 = deserialize_arg(&args, 1);

    let owner = sender();
    ALLOWANCES.set(&owner, &spender, &amount);

    emit_event(b"Approval", &bincode::serialize(&(owner, spender, amount)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn mint(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let to: Address = deserialize_arg(&args, 0);
    let amount: u128 = deserialize_arg(&args, 1);

    check_role(minter_role(), sender());

    let supply = TOTAL_SUPPLY.get_or_default();
    let token_cap = CAP.get_or_default();
    let new_supply = add(supply, amount);
    if new_supply > token_cap {
        revert("ERC20Capped: cap exceeded");
    }

    TOTAL_SUPPLY.set(&new_supply);
    BALANCES.set(&to, &add(BALANCES.get_or_default(&to), amount));

    emit_event(b"Transfer", &bincode::serialize(&([0u8; 32], to, amount)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn burn(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let amount: u128 = deserialize_arg(&args, 0);

    let owner = sender();
    let balance_owner = BALANCES.get_or_default(&owner);
    if balance_owner < amount {
        revert("ERC20: burn amount exceeds balance");
    }

    BALANCES.set(&owner, &sub(balance_owner, amount));
    TOTAL_SUPPLY.set(&sub(TOTAL_SUPPLY.get_or_default(), amount));

    emit_event(b"Transfer", &bincode::serialize(&(owner, [0u8; 32], amount)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn pause(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    check_role(pauser_role(), sender());
    PAUSED.set(&true);
    emit_event(b"Paused", &[]);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn unpause(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    check_role(pauser_role(), sender());
    PAUSED.set(&false);
    emit_event(b"Unpaused", &[]);
    return_data(&true)
}

// Queries
#[no_mangle]
pub extern "C" fn balance_of(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let owner: Address = deserialize_arg(&args, 0);
    let bal = BALANCES.get_or_default(&owner);
    return_data(&bal)
}

#[no_mangle]
pub extern "C" fn allowance(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let owner: Address = deserialize_arg(&args, 0);
    let spender: Address = deserialize_arg(&args, 1);
    let allowed = ALLOWANCES.get_or_default(&owner, &spender);
    return_data(&allowed)
}

#[no_mangle]
pub extern "C" fn total_supply(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    let supply = TOTAL_SUPPLY.get_or_default();
    return_data(&supply)
}

#[no_mangle]
pub extern "C" fn cap(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    let token_cap = CAP.get_or_default();
    return_data(&token_cap)
}

#[no_mangle]
pub extern "C" fn paused(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    let is_paused = PAUSED.get_or_default();
    return_data(&is_paused)
}

// AccessControl RBAC
#[no_mangle]
pub extern "C" fn grant_role_entry(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let role: [u8; 32] = deserialize_arg(&args, 0);
    let account: Address = deserialize_arg(&args, 1);

    check_role(DEFAULT_ADMIN_ROLE, sender());
    grant_role(role, account);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn revoke_role_entry(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let role: [u8; 32] = deserialize_arg(&args, 0);
    let account: Address = deserialize_arg(&args, 1);

    check_role(DEFAULT_ADMIN_ROLE, sender());
    revoke_role(role, account);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn has_role_entry(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let role: [u8; 32] = deserialize_arg(&args, 0);
    let account: Address = deserialize_arg(&args, 1);
    let res = resurgence_common::auth::has_role(role, account);
    return_data(&res)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn transferFrom(ptr: i32, len: i32) -> i32 {
    transfer_from(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn balanceOf(ptr: i32, len: i32) -> i32 {
    balance_of(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn totalSupply(ptr: i32, len: i32) -> i32 {
    total_supply(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn grantRole(ptr: i32, len: i32) -> i32 {
    grant_role_entry(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn revokeRole(ptr: i32, len: i32) -> i32 {
    revoke_role_entry(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn hasRole(ptr: i32, len: i32) -> i32 {
    has_role_entry(ptr, len)
}
