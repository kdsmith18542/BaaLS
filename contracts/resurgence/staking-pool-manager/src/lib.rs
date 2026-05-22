use resurgence_common::auth::{
    check_role, emergency_pauser_role, grant_role, timelock_role, DEFAULT_ADMIN_ROLE,
};
use resurgence_common::host::{
    call_contract, deserialize_arg, emit_event, get_input_args, return_data, revert, sender,
};
use resurgence_common::math::{mul, sub};
use resurgence_common::storage::{StorageMap, StorageValue};
use resurgence_common::types::Address;

const INITIALIZED: StorageValue<bool> = StorageValue::new(b"initialized");
const RESURGENCE_TOKEN: StorageValue<Address> = StorageValue::new(b"token");
const REWARD_DISTRIBUTOR: StorageValue<Address> = StorageValue::new(b"reward_distributor");

const DYNAMIC_RATE_ENABLED: StorageValue<bool> = StorageValue::new(b"dynamic_rate_enabled");
const BASE_REWARD_RATE: StorageValue<u128> = StorageValue::new(b"base_reward_rate");
const TVL_DECAY_FACTOR: StorageValue<u128> = StorageValue::new(b"tvl_decay_factor");
const MIN_REWARD_RATE: StorageValue<u128> = StorageValue::new(b"min_reward_rate");
const MAX_REWARD_RATE: StorageValue<u128> = StorageValue::new(b"max_reward_rate");

const DEAD_COIN_TO_POOL: StorageMap<Address, Address> = StorageMap::new(b"dead_coin_to_pool");
const SUPPORTED_DEAD_COINS: StorageValue<Vec<Address>> = StorageValue::new(b"supported_dead_coins");
const PAUSED: StorageValue<bool> = StorageValue::new(b"paused");

#[no_mangle]
pub extern "C" fn initialize(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let token_address: Address = deserialize_arg(&args, 0);
    let distributor_address: Address = deserialize_arg(&args, 1);
    let timelock: Address = deserialize_arg(&args, 2);

    if INITIALIZED.get_or_default() {
        revert("Already initialized");
    }
    if token_address == [0u8; 32] || distributor_address == [0u8; 32] || timelock == [0u8; 32] {
        revert("Invalid addresses");
    }

    INITIALIZED.set(&true);
    RESURGENCE_TOKEN.set(&token_address);
    REWARD_DISTRIBUTOR.set(&distributor_address);

    DYNAMIC_RATE_ENABLED.set(&false);
    BASE_REWARD_RATE.set(&1_000_000_000_000_000_000u128); // 1 RESURGE per second
    TVL_DECAY_FACTOR.set(&100); // 100 bps (1%) decay per 1M TVL
    MIN_REWARD_RATE.set(&100_000_000_000_000_000u128); // 0.1 RESURGE
    MAX_REWARD_RATE.set(&10_000_000_000_000_000_000u128); // 10 RESURGE

    SUPPORTED_DEAD_COINS.set(&Vec::new());

    grant_role(DEFAULT_ADMIN_ROLE, timelock);
    grant_role(timelock_role(), timelock);
    grant_role(emergency_pauser_role(), timelock);

    // Grant temporary roles to deployer for setup
    let caller = sender();
    grant_role(DEFAULT_ADMIN_ROLE, caller);
    grant_role(timelock_role(), caller);

    return_data(&true)
}

#[no_mangle]
pub extern "C" fn add_staking_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);
    let pool_contract: Address = deserialize_arg(&args, 1);
    let initial_rate: u128 = deserialize_arg(&args, 2);

    check_role(timelock_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    if dead_coin == [0u8; 32] || pool_contract == [0u8; 32] {
        revert("Invalid addresses");
    }

    if DEAD_COIN_TO_POOL.get(&dead_coin).is_some() {
        revert("StakingPoolManager: pool already exists");
    }

    DEAD_COIN_TO_POOL.set(&dead_coin, &pool_contract);

    let mut coins = SUPPORTED_DEAD_COINS.get_or_default();
    coins.push(dead_coin);
    SUPPORTED_DEAD_COINS.set(&coins);

    // Authorize pool on the distributor
    let distributor = REWARD_DISTRIBUTOR.get().unwrap_or_else(|| revert("Distributor not set"));
    let call_args1 = vec![bincode::serialize(&pool_contract).unwrap()];
    let serialized_call_args1 = bincode::serialize(&call_args1).unwrap();
    let res1 = call_contract(&distributor, "authorize_pool", &serialized_call_args1, 0);
    if res1 < 0 {
        revert("StakingPoolManager: authorize_pool failed");
    }

    // Set initial reward rate on pool if > 0
    if initial_rate > 0 {
        let call_args2 = vec![bincode::serialize(&initial_rate).unwrap()];
        let serialized_call_args2 = bincode::serialize(&call_args2).unwrap();
        let res2 = call_contract(&pool_contract, "set_reward_rate", &serialized_call_args2, 0);
        if res2 < 0 {
            revert("StakingPoolManager: set_reward_rate failed");
        }
    }

    emit_event(
        b"StakingPoolAdded",
        &bincode::serialize(&(dead_coin, pool_contract, initial_rate)).unwrap(),
    );
    return_data(&pool_contract)
}

#[no_mangle]
pub extern "C" fn set_reward_rate(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);
    let new_rate: u128 = deserialize_arg(&args, 1);

    check_role(timelock_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    let pool = DEAD_COIN_TO_POOL
        .get(&dead_coin)
        .unwrap_or_else(|| revert("StakingPoolManager: pool not found"));
    let call_args = vec![bincode::serialize(&new_rate).unwrap()];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&pool, "set_reward_rate", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPoolManager: call failed");
    }

    emit_event(b"RewardRateUpdated", &bincode::serialize(&(dead_coin, new_rate)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn pause_staking_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    let pool = DEAD_COIN_TO_POOL
        .get(&dead_coin)
        .unwrap_or_else(|| revert("StakingPoolManager: pool not found"));
    let call_args: Vec<Vec<u8>> = Vec::new();
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&pool, "pause", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPoolManager: call failed");
    }

    emit_event(b"StakingPoolPaused", &bincode::serialize(&(dead_coin, pool)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn unpause_staking_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    let pool = DEAD_COIN_TO_POOL
        .get(&dead_coin)
        .unwrap_or_else(|| revert("StakingPoolManager: pool not found"));
    let call_args: Vec<Vec<u8>> = Vec::new();
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&pool, "unpause", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPoolManager: call failed");
    }

    emit_event(b"StakingPoolUnpaused", &bincode::serialize(&(dead_coin, pool)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn remove_staking_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    let pool = DEAD_COIN_TO_POOL
        .get(&dead_coin)
        .unwrap_or_else(|| revert("StakingPoolManager: pool not found"));

    let distributor = REWARD_DISTRIBUTOR.get().unwrap_or_else(|| revert("Distributor not set"));
    let call_args = vec![bincode::serialize(&pool).unwrap()];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&distributor, "unauthorize_pool", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPoolManager: call failed");
    }

    DEAD_COIN_TO_POOL.remove(&dead_coin);

    let mut coins = SUPPORTED_DEAD_COINS.get_or_default();
    if let Some(pos) = coins.iter().position(|x| *x == dead_coin) {
        coins.swap_remove(pos);
    }
    SUPPORTED_DEAD_COINS.set(&coins);

    emit_event(b"StakingPoolRemoved", &bincode::serialize(&(dead_coin, pool)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_dynamic_rate_enabled(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let enabled: bool = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    DYNAMIC_RATE_ENABLED.set(&enabled);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_dynamic_rate_params(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let base_rate: u128 = deserialize_arg(&args, 0);
    let decay_factor: u128 = deserialize_arg(&args, 1);
    let min_rate: u128 = deserialize_arg(&args, 2);
    let max_rate: u128 = deserialize_arg(&args, 3);

    check_role(timelock_role(), sender());
    BASE_REWARD_RATE.set(&base_rate);
    TVL_DECAY_FACTOR.set(&decay_factor);
    MIN_REWARD_RATE.set(&min_rate);
    MAX_REWARD_RATE.set(&max_rate);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn calculate_dynamic_rate(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let tvl: u128 = deserialize_arg(&args, 0);
    let multiplier: u128 = deserialize_arg(&args, 1);

    if !DYNAMIC_RATE_ENABLED.get_or_default() {
        return return_data(&BASE_REWARD_RATE.get_or_default());
    }

    let tvl_millions = tvl / 1_000_000_000_000_000_000_000_000u128; // tvl / 10^24 (1M * 1e18)
    let base = BASE_REWARD_RATE.get_or_default();
    let decay = TVL_DECAY_FACTOR.get_or_default();
    let reduction = if tvl_millions > 0 { mul(base, mul(tvl_millions, decay)) / 10000 } else { 0 };

    let mut dynamic_rate = base;
    if reduction < dynamic_rate {
        dynamic_rate = sub(dynamic_rate, reduction);
    } else {
        dynamic_rate = MIN_REWARD_RATE.get_or_default();
    }

    let max_rate = MAX_REWARD_RATE.get_or_default();
    let min_rate = MIN_REWARD_RATE.get_or_default();

    if dynamic_rate > max_rate {
        dynamic_rate = max_rate;
    }
    if dynamic_rate < min_rate {
        dynamic_rate = min_rate;
    }

    // Apply multiplier
    dynamic_rate = mul(dynamic_rate, multiplier) / 10000;

    return_data(&dynamic_rate)
}

#[no_mangle]
pub extern "C" fn apply_dynamic_rate(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);
    let tvl: u128 = deserialize_arg(&args, 1);
    let multiplier: u128 = deserialize_arg(&args, 2);

    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    let pool = DEAD_COIN_TO_POOL
        .get(&dead_coin)
        .unwrap_or_else(|| revert("StakingPoolManager: pool not found"));

    // Call calculate_dynamic_rate logic directly
    let mut reduction = 0u128;
    let base = BASE_REWARD_RATE.get_or_default();
    if DYNAMIC_RATE_ENABLED.get_or_default() {
        let tvl_millions = tvl / 1_000_000_000_000_000_000_000_000u128;
        let decay = TVL_DECAY_FACTOR.get_or_default();
        if tvl_millions > 0 {
            reduction = mul(base, mul(tvl_millions, decay)) / 10000;
        }
    }

    let mut dynamic_rate = base;
    if reduction < dynamic_rate {
        dynamic_rate = sub(dynamic_rate, reduction);
    } else {
        dynamic_rate = MIN_REWARD_RATE.get_or_default();
    }

    let max_rate = MAX_REWARD_RATE.get_or_default();
    let min_rate = MIN_REWARD_RATE.get_or_default();

    if dynamic_rate > max_rate {
        dynamic_rate = max_rate;
    }
    if dynamic_rate < min_rate {
        dynamic_rate = min_rate;
    }

    dynamic_rate = mul(dynamic_rate, multiplier) / 10000;

    let call_args = vec![bincode::serialize(&dynamic_rate).unwrap()];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&pool, "set_reward_rate", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPoolManager: call failed");
    }

    emit_event(b"RewardRateUpdated", &bincode::serialize(&(dead_coin, dynamic_rate)).unwrap());
    return_data(&dynamic_rate)
}

#[no_mangle]
pub extern "C" fn batch_stake(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coins: Vec<Address> = deserialize_arg(&args, 0);
    let amounts: Vec<u128> = deserialize_arg(&args, 1);

    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    if dead_coins.len() != amounts.len() {
        revert("Mismatched arrays");
    }
    if dead_coins.len() > 50 {
        revert("Batch size too large");
    }

    let caller = sender();

    for i in 0..dead_coins.len() {
        let amt = amounts[i];
        if amt > 0 {
            let dead_coin = dead_coins[i];
            let pool = DEAD_COIN_TO_POOL
                .get(&dead_coin)
                .unwrap_or_else(|| revert("StakingPoolManager: pool not found"));

            // Pull tokens from caller directly into the pool
            let transfer_from_args = vec![
                bincode::serialize(&caller).unwrap(),
                bincode::serialize(&pool).unwrap(),
                bincode::serialize(&amt).unwrap(),
            ];
            let transfer_from_payload = bincode::serialize(&transfer_from_args).unwrap();
            let transfer_res =
                call_contract(&dead_coin, "transfer_from", &transfer_from_payload, 0);
            if transfer_res < 0 {
                revert("StakingPoolManager: transfer_from failed");
            }

            // Queue stake_for(caller, amt) on pool
            let call_args =
                vec![bincode::serialize(&caller).unwrap(), bincode::serialize(&amt).unwrap()];
            let serialized_call_args = bincode::serialize(&call_args).unwrap();
            let res = call_contract(&pool, "stake_for", &serialized_call_args, 0);
            if res < 0 {
                revert("StakingPoolManager: batch_stake failed");
            }
        }
    }

    return_data(&true)
}

#[no_mangle]
pub extern "C" fn batch_claim_rewards(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coins: Vec<Address> = deserialize_arg(&args, 0);

    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    if dead_coins.len() > 50 {
        revert("Batch size too large");
    }

    let caller = sender();
    for i in 0..dead_coins.len() {
        let dead_coin = dead_coins[i];
        let Some(pool) = DEAD_COIN_TO_POOL.get(&dead_coin) else {
            continue;
        };

        // Queue claim_rewards_for(caller) on pool
        let call_args = vec![bincode::serialize(&caller).unwrap()];
        let serialized_call_args = bincode::serialize(&call_args).unwrap();
        let res = call_contract(&pool, "claim_rewards_for", &serialized_call_args, 0);
        if res < 0 {
            revert("StakingPoolManager: batch_claim failed");
        }
    }

    return_data(&true)
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

// Queries
#[no_mangle]
pub extern "C" fn get_staking_pool(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin: Address = deserialize_arg(&args, 0);
    let pool = DEAD_COIN_TO_POOL.get(&dead_coin).unwrap_or([0u8; 32]);
    return_data(&pool)
}

#[no_mangle]
pub extern "C" fn supported_dead_coins(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&SUPPORTED_DEAD_COINS.get_or_default())
}

#[no_mangle]
pub extern "C" fn is_paused(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&PAUSED.get_or_default())
}

#[no_mangle]
pub extern "C" fn dynamic_rate_enabled(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&DYNAMIC_RATE_ENABLED.get_or_default())
}
