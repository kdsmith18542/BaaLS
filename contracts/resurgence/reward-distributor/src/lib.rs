use resurgence_common::auth::{
    check_role, emergency_pauser_role, grant_role, oracle_manager_role, revoke_role, timelock_role,
    DEFAULT_ADMIN_ROLE,
};
use resurgence_common::host::{
    baals_call_contract, deserialize_arg, emit_event, get_input_args, return_data, revert, sender,
};
use resurgence_common::math::add;
use resurgence_common::storage::{StorageMap, StorageValue};
use resurgence_common::types::Address;

const INITIALIZED: StorageValue<bool> = StorageValue::new(b"initialized");
const RESURGENCE_TOKEN: StorageValue<Address> = StorageValue::new(b"token");
const TOTAL_MINTED: StorageValue<u128> = StorageValue::new(b"total_minted");
const MAX_MINT_SUPPLY: StorageValue<u128> = StorageValue::new(b"max_mint_supply");
const AUTHORIZED_POOLS: StorageMap<Address, bool> = StorageMap::new(b"pools");
const ORACLE_ADDRESS: StorageValue<Address> = StorageValue::new(b"oracle_address");
const ORACLE_STALE_THRESHOLD: StorageValue<u64> = StorageValue::new(b"oracle_stale_threshold");
const ORACLE_ENABLED: StorageValue<bool> = StorageValue::new(b"oracle_enabled");
const ORACLE_LAST_PRICE: StorageValue<u128> = StorageValue::new(b"oracle_price");
const ORACLE_LAST_UPDATE: StorageValue<u64> = StorageValue::new(b"oracle_update");
const PAUSED: StorageValue<bool> = StorageValue::new(b"paused");

fn current_price_and_valid() -> (u128, bool) {
    let stale_threshold = {
        let configured = ORACLE_STALE_THRESHOLD.get_or_default();
        if configured == 0 { 3600 } else { configured }
    };
    let now = resurgence_common::host::block_timestamp();

    // Current runtime does not support synchronous oracle reads from another contract.
    // Keep oracle configuration for ABI compatibility and rely on fallback price storage.
    let _oracle_enabled = ORACLE_ENABLED.get_or_default();
    let _oracle_address = ORACLE_ADDRESS.get().unwrap_or([0u8; 32]);

    let price = ORACLE_LAST_PRICE.get_or_default();
    let last_update = ORACLE_LAST_UPDATE.get_or_default();
    let valid = price > 0 && now < last_update.saturating_add(stale_threshold);
    (price, valid)
}

#[no_mangle]
pub extern "C" fn initialize(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let token_address: Address = deserialize_arg(&args, 0);
    let initial_max_mint: u128 = deserialize_arg(&args, 1);
    let timelock: Address = deserialize_arg(&args, 2);

    if INITIALIZED.get_or_default() {
        revert("Already initialized");
    }
    INITIALIZED.set(&true);
    RESURGENCE_TOKEN.set(&token_address);
    MAX_MINT_SUPPLY.set(&initial_max_mint);

    grant_role(DEFAULT_ADMIN_ROLE, timelock);
    grant_role(timelock_role(), timelock);
    grant_role(emergency_pauser_role(), timelock);
    grant_role(oracle_manager_role(), timelock);

    // Grant temporary roles to deployer for setup
    let caller = sender();
    grant_role(DEFAULT_ADMIN_ROLE, caller);
    grant_role(timelock_role(), caller);
    grant_role(oracle_manager_role(), caller);
    ORACLE_STALE_THRESHOLD.set(&3600);
    ORACLE_ENABLED.set(&false);

    return_data(&true)
}

#[no_mangle]
pub extern "C" fn authorize_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let pool: Address = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if pool == [0u8; 32] {
        revert("Invalid address");
    }
    AUTHORIZED_POOLS.set(&pool, &true);
    emit_event(b"StakingPoolAuthorized", &bincode::serialize(&pool).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn unauthorize_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let pool: Address = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if pool == [0u8; 32] {
        revert("Invalid address");
    }
    AUTHORIZED_POOLS.set(&pool, &false);
    emit_event(b"StakingPoolDeauthorized", &bincode::serialize(&pool).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn mint_and_distribute(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let caller_pool = sender();
    let (to, amount) = match args.len() {
        2 => {
            let to: Address = deserialize_arg(&args, 0);
            let amount: u128 = deserialize_arg(&args, 1);
            (to, amount)
        }
        3 => {
            // Backward-compatible ABI path: enforce claimed pool identity matches caller.
            let claimed_pool: Address = deserialize_arg(&args, 0);
            if claimed_pool != caller_pool {
                revert("RewardDistributor: caller pool mismatch");
            }
            let to: Address = deserialize_arg(&args, 1);
            let amount: u128 = deserialize_arg(&args, 2);
            (to, amount)
        }
        _ => revert("RewardDistributor: invalid argument count"),
    };

    if !AUTHORIZED_POOLS.get_or_default(&caller_pool) {
        revert("RewardDistributor: unauthorized pool");
    }

    let total = TOTAL_MINTED.get_or_default();
    let max_supply = MAX_MINT_SUPPLY.get_or_default();
    let new_total = add(total, amount);
    if new_total > max_supply {
        revert("RewardDistributor: exceeds max supply");
    }

    TOTAL_MINTED.set(&new_total);

    let token_address = RESURGENCE_TOKEN.get().unwrap_or_else(|| revert("Token not set"));
    let call_args = vec![bincode::serialize(&to).unwrap(), bincode::serialize(&amount).unwrap()];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();

    unsafe {
        let call_idx = baals_call_contract(
            token_address.as_ptr() as i32,
            32,
            b"mint".as_ptr() as i32,
            4,
            serialized_call_args.as_ptr() as i32,
            serialized_call_args.len() as i32,
            0i64,
        );
        if call_idx < 0 {
            revert("RewardDistributor: call to mint failed");
        }
    }

    emit_event(b"TokensMintedAndDistributed", &bincode::serialize(&(to, amount)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_max_mint_supply(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let new_max: u128 = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if new_max < TOTAL_MINTED.get_or_default() {
        revert("Supply too low");
    }
    MAX_MINT_SUPPLY.set(&new_max);
    emit_event(b"MaxMintSupplyUpdated", &bincode::serialize(&new_max).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_fallback_price(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let price: u128 = deserialize_arg(&args, 0);

    let caller = sender();
    if !resurgence_common::auth::has_role(oracle_manager_role(), caller)
        && !resurgence_common::auth::has_role(timelock_role(), caller)
    {
        revert("AccessControl: sender missing oracle manager or timelock role");
    }

    ORACLE_LAST_PRICE.set(&price);
    ORACLE_LAST_UPDATE.set(&resurgence_common::host::block_timestamp());

    emit_event(
        b"OraclePriceUpdated",
        &bincode::serialize(&(price, resurgence_common::host::block_timestamp())).unwrap(),
    );
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_price_oracle(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let oracle: Address = deserialize_arg(&args, 0);
    let stale_threshold: u128 = deserialize_arg(&args, 1);

    check_role(timelock_role(), sender());
    if oracle == [0u8; 32] {
        revert("Invalid address");
    }
    let stale = if stale_threshold == 0 {
        3600u64
    } else {
        u64::try_from(stale_threshold).unwrap_or_else(|_| revert("Stale threshold too large"))
    };

    ORACLE_ADDRESS.set(&oracle);
    ORACLE_STALE_THRESHOLD.set(&stale);
    ORACLE_ENABLED.set(&true);
    emit_event(
        b"PriceOracleSet",
        &bincode::serialize(&(oracle, ORACLE_STALE_THRESHOLD.get_or_default())).unwrap(),
    );
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_oracle_enabled(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let enabled: bool = deserialize_arg(&args, 0);
    check_role(timelock_role(), sender());
    ORACLE_ENABLED.set(&enabled);
    emit_event(b"OracleEnabled", &bincode::serialize(&enabled).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn get_resurge_price(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    let (price, valid) = current_price_and_valid();
    return_data(&(price, valid))
}

#[no_mangle]
pub extern "C" fn get_emission_multiplier(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    let (price, valid) = current_price_and_valid();

    let base_price: u128 = 5_000_000;

    if !valid {
        return return_data(&10000u128);
    }

    if price <= base_price {
        return return_data(&10000u128);
    }

    let excess = price - base_price;
    let mut multiplier = 10000 + (excess * 1000) / 1_000_000;
    if multiplier > 20000 {
        multiplier = 20000;
    }

    return_data(&multiplier)
}

#[no_mangle]
pub extern "C" fn pause(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    check_role(emergency_pauser_role(), sender());
    PAUSED.set(&true);
    emit_event(b"Paused", &[]);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn unpause(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    check_role(DEFAULT_ADMIN_ROLE, sender());
    PAUSED.set(&false);
    emit_event(b"Unpaused", &[]);
    return_data(&true)
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

// Queries
#[no_mangle]
pub extern "C" fn total_resurge_minted(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&TOTAL_MINTED.get_or_default())
}

#[no_mangle]
pub extern "C" fn max_mint_supply(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&MAX_MINT_SUPPLY.get_or_default())
}

#[no_mangle]
pub extern "C" fn is_authorized_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let pool: Address = deserialize_arg(&args, 0);
    return_data(&AUTHORIZED_POOLS.get_or_default(&pool))
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn authorizeStakingPool(ptr: i32, len: i32) -> i32 {
    authorize_pool(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn unauthorizeStakingPool(ptr: i32, len: i32) -> i32 {
    unauthorize_pool(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn mintAndDistribute(ptr: i32, len: i32) -> i32 {
    mint_and_distribute(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn addAuthorizedStakingPool(ptr: i32, len: i32) -> i32 {
    authorize_pool(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn removeAuthorizedStakingPool(ptr: i32, len: i32) -> i32 {
    unauthorize_pool(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn setMaxMintSupply(ptr: i32, len: i32) -> i32 {
    set_max_mint_supply(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn setFallbackPrice(ptr: i32, len: i32) -> i32 {
    set_fallback_price(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn setPriceOracle(ptr: i32, len: i32) -> i32 {
    set_price_oracle(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn setOracleEnabled(ptr: i32, len: i32) -> i32 {
    set_oracle_enabled(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn getResurgePrice(ptr: i32, len: i32) -> i32 {
    get_resurge_price(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn getEmissionMultiplier(ptr: i32, len: i32) -> i32 {
    get_emission_multiplier(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn totalResurgeMinted(ptr: i32, len: i32) -> i32 {
    total_resurge_minted(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn maxMintSupply(ptr: i32, len: i32) -> i32 {
    max_mint_supply(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn authorizedStakingPools(ptr: i32, len: i32) -> i32 {
    is_authorized_pool(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn resurgenceToken(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&RESURGENCE_TOKEN.get().unwrap_or([0u8; 32]))
}
