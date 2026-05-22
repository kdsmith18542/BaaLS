use resurgence_common::auth::{
    check_role, emergency_pauser_role, grant_role, timelock_role, DEFAULT_ADMIN_ROLE,
};
use resurgence_common::host::{
    call_contract, contract_id, deserialize_arg, emit_event, get_input_args, return_data, revert,
    sender,
};
use resurgence_common::math::{add, mul, sub};
use resurgence_common::storage::{StorageMap, StorageValue};
use resurgence_common::types::Address;

const INITIALIZED: StorageValue<bool> = StorageValue::new(b"initialized");
const DEAD_COIN: StorageValue<Address> = StorageValue::new(b"dead_coin");
const RESURGENCE_TOKEN: StorageValue<Address> = StorageValue::new(b"resurgence_token");
const REWARD_DISTRIBUTOR: StorageValue<Address> = StorageValue::new(b"reward_distributor");
const STAKING_POOL_MANAGER: StorageValue<Address> = StorageValue::new(b"staking_pool_manager");

const REWARD_RATE_PER_SECOND: StorageValue<u128> = StorageValue::new(b"reward_rate");
const LAST_UPDATE_TIME: StorageValue<u64> = StorageValue::new(b"last_update");
const REWARD_PER_TOKEN_STORED: StorageValue<u128> = StorageValue::new(b"reward_per_token_stored");

const USER_STAKED_AMOUNT: StorageMap<Address, u128> = StorageMap::new(b"user_staked");
const USER_REWARD_PER_TOKEN_PAID: StorageMap<Address, u128> = StorageMap::new(b"user_paid");
const USER_REWARDS: StorageMap<Address, u128> = StorageMap::new(b"user_rewards");
const TOTAL_STAKED_SUPPLY: StorageValue<u128> = StorageValue::new(b"total_staked");
const PAUSED: StorageValue<bool> = StorageValue::new(b"paused");

#[no_mangle]
pub extern "C" fn initialize(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let dead_coin_address: Address = deserialize_arg(&args, 0);
    let resurgence_token_address: Address = deserialize_arg(&args, 1);
    let reward_distributor_address: Address = deserialize_arg(&args, 2);
    let staking_pool_manager_address: Address = deserialize_arg(&args, 3);
    let timelock: Address = deserialize_arg(&args, 4);

    if INITIALIZED.get_or_default() {
        revert("Already initialized");
    }
    if dead_coin_address == [0u8; 32]
        || resurgence_token_address == [0u8; 32]
        || reward_distributor_address == [0u8; 32]
        || staking_pool_manager_address == [0u8; 32]
        || timelock == [0u8; 32]
    {
        revert("Invalid addresses");
    }

    INITIALIZED.set(&true);
    DEAD_COIN.set(&dead_coin_address);
    RESURGENCE_TOKEN.set(&resurgence_token_address);
    REWARD_DISTRIBUTOR.set(&reward_distributor_address);
    STAKING_POOL_MANAGER.set(&staking_pool_manager_address);

    grant_role(DEFAULT_ADMIN_ROLE, timelock);
    grant_role(timelock_role(), timelock);
    grant_role(emergency_pauser_role(), timelock);

    grant_role(timelock_role(), staking_pool_manager_address);

    LAST_UPDATE_TIME.set(&resurgence_common::host::block_timestamp());

    return_data(&true)
}

fn reward_per_token() -> u128 {
    let total_supply = TOTAL_STAKED_SUPPLY.get_or_default();
    let rate = REWARD_RATE_PER_SECOND.get_or_default();
    let stored = REWARD_PER_TOKEN_STORED.get_or_default();
    if total_supply == 0 || rate == 0 {
        return stored;
    }
    let now = resurgence_common::host::block_timestamp();
    let last_update = LAST_UPDATE_TIME.get_or_default();
    if now <= last_update {
        return stored;
    }
    let duration = (now - last_update) as u128;
    let reward_increase = mul(duration, rate);
    let reward_increase_scaled = mul(reward_increase, 1_000_000_000_000_000_000u128); // 1e18
    let reward_per_token_increase = reward_increase_scaled / total_supply;
    add(stored, reward_per_token_increase)
}

fn earned(account: Address) -> u128 {
    let staked = USER_STAKED_AMOUNT.get_or_default(&account);
    let paid = USER_REWARD_PER_TOKEN_PAID.get_or_default(&account);
    let current_rpt = reward_per_token();
    let diff = sub(current_rpt, paid);
    let pending_rewards_scaled = mul(staked, diff);
    let pending_rewards = pending_rewards_scaled / 1_000_000_000_000_000_000u128;
    add(USER_REWARDS.get_or_default(&account), pending_rewards)
}

fn update_reward(account: Option<Address>) {
    let rpt = reward_per_token();
    REWARD_PER_TOKEN_STORED.set(&rpt);
    LAST_UPDATE_TIME.set(&resurgence_common::host::block_timestamp());
    if let Some(acc) = account {
        if acc != [0u8; 32] {
            let earnings = earned(acc);
            USER_REWARDS.set(&acc, &earnings);
            USER_REWARD_PER_TOKEN_PAID.set(&acc, &rpt);
        }
    }
}

fn stake_internal(staker: Address, amount: u128) {
    if amount == 0 {
        revert("Invalid amount");
    }
    let dead_coin_addr = DEAD_COIN.get().unwrap_or_else(|| revert("Dead coin not set"));

    let call_args = vec![
        bincode::serialize(&sender()).unwrap(),
        bincode::serialize(&contract_id()).unwrap(),
        bincode::serialize(&amount).unwrap(),
    ];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&dead_coin_addr, "transfer_from", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPool: transfer_from failed");
    }

    credit_stake(staker, amount);
}

fn credit_stake(staker: Address, amount: u128) {
    let staked = USER_STAKED_AMOUNT.get_or_default(&staker);
    USER_STAKED_AMOUNT.set(&staker, &add(staked, amount));

    let total_staked = TOTAL_STAKED_SUPPLY.get_or_default();
    TOTAL_STAKED_SUPPLY.set(&add(total_staked, amount));

    emit_event(b"Staked", &bincode::serialize(&(staker, amount)).unwrap());
}

#[no_mangle]
pub extern "C" fn stake(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let amount: u128 = deserialize_arg(&args, 0);

    let caller = sender();
    update_reward(Some(caller));
    stake_internal(caller, amount);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn stake_for(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    let amount: u128 = deserialize_arg(&args, 1);

    let manager =
        STAKING_POOL_MANAGER.get().unwrap_or_else(|| revert("Staking pool manager not set"));
    if sender() != manager {
        revert("StakingPool: only manager can call stake_for");
    }
    if amount == 0 {
        revert("Invalid amount");
    }

    update_reward(Some(user));
    credit_stake(user, amount);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn unstake(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let amount: u128 = deserialize_arg(&args, 0);

    let caller = sender();
    update_reward(Some(caller));

    if amount == 0 {
        revert("Invalid amount");
    }
    let staked = USER_STAKED_AMOUNT.get_or_default(&caller);
    if staked < amount {
        revert("Insufficient balance");
    }

    USER_STAKED_AMOUNT.set(&caller, &sub(staked, amount));

    let total_staked = TOTAL_STAKED_SUPPLY.get_or_default();
    TOTAL_STAKED_SUPPLY.set(&sub(total_staked, amount));

    let dead_coin_addr = DEAD_COIN.get().unwrap_or_else(|| revert("Dead coin not set"));
    let call_args =
        vec![bincode::serialize(&caller).unwrap(), bincode::serialize(&amount).unwrap()];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&dead_coin_addr, "transfer", &serialized_call_args, 0);
    if res < 0 {
        revert("StakingPool: transfer failed");
    }

    emit_event(b"Unstaked", &bincode::serialize(&(caller, amount)).unwrap());
    return_data(&true)
}

fn claim_rewards_internal(user: Address) {
    let rewards = USER_REWARDS.get_or_default(&user);
    if rewards > 0 {
        USER_REWARDS.set(&user, &0);
        let distributor = REWARD_DISTRIBUTOR.get().unwrap_or_else(|| revert("Distributor not set"));
        let call_args = vec![
            bincode::serialize(&contract_id()).unwrap(),
            bincode::serialize(&user).unwrap(),
            bincode::serialize(&rewards).unwrap(),
        ];
        let serialized_call_args = bincode::serialize(&call_args).unwrap();
        let res = call_contract(&distributor, "mint_and_distribute", &serialized_call_args, 0);
        if res < 0 {
            revert("StakingPool: call to mint_and_distribute failed");
        }
        emit_event(b"RewardsClaimed", &bincode::serialize(&(user, rewards)).unwrap());
    }
}

#[no_mangle]
pub extern "C" fn claim_rewards(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let _args = get_input_args(len);
    let caller = sender();
    update_reward(Some(caller));
    claim_rewards_internal(caller);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn claim_rewards_for(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    update_reward(Some(user));
    claim_rewards_internal(user);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn claim_and_restake_to(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let resurge_staking_pool: Address = deserialize_arg(&args, 0);

    let caller = sender();
    update_reward(Some(caller));

    let rewards = USER_REWARDS.get_or_default(&caller);
    if rewards == 0 {
        revert("No rewards to claim");
    }

    USER_REWARDS.set(&caller, &0);

    let distributor = REWARD_DISTRIBUTOR.get().unwrap_or_else(|| revert("Distributor not set"));
    let this_address = contract_id();
    let call_args1 = vec![
        bincode::serialize(&this_address).unwrap(),
        bincode::serialize(&this_address).unwrap(),
        bincode::serialize(&rewards).unwrap(),
    ];
    let serialized_call_args1 = bincode::serialize(&call_args1).unwrap();
    let res1 = call_contract(&distributor, "mint_and_distribute", &serialized_call_args1, 0);
    if res1 < 0 {
        revert("StakingPool: mint_and_distribute failed");
    }

    let resurgence_token =
        RESURGENCE_TOKEN.get().unwrap_or_else(|| revert("Resurgence token not set"));
    let call_args2 = vec![
        bincode::serialize(&resurge_staking_pool).unwrap(),
        bincode::serialize(&rewards).unwrap(),
    ];
    let serialized_call_args2 = bincode::serialize(&call_args2).unwrap();
    let res2 = call_contract(&resurgence_token, "approve", &serialized_call_args2, 0);
    if res2 < 0 {
        revert("StakingPool: approve failed");
    }

    let call_args3 = vec![
        bincode::serialize(&this_address).unwrap(),
        bincode::serialize(&caller).unwrap(),
        bincode::serialize(&rewards).unwrap(),
    ];
    let serialized_call_args3 = bincode::serialize(&call_args3).unwrap();
    let res3 = call_contract(&resurge_staking_pool, "stake_for", &serialized_call_args3, 0);
    if res3 < 0 {
        revert("StakingPool: stake_for failed");
    }

    emit_event(b"RewardsClaimed", &bincode::serialize(&(caller, rewards)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_reward_rate(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let new_rate: u128 = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    update_reward(None);
    REWARD_RATE_PER_SECOND.set(&new_rate);

    emit_event(b"RewardRateUpdated", &bincode::serialize(&new_rate).unwrap());
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
pub extern "C" fn earned_query(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    return_data(&earned(user))
}

#[no_mangle]
pub extern "C" fn user_staked_amount(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    return_data(&USER_STAKED_AMOUNT.get_or_default(&user))
}

#[no_mangle]
pub extern "C" fn total_staked_supply(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&TOTAL_STAKED_SUPPLY.get_or_default())
}

#[no_mangle]
pub extern "C" fn reward_rate_per_second(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&REWARD_RATE_PER_SECOND.get_or_default())
}

#[no_mangle]
pub extern "C" fn is_paused(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&PAUSED.get_or_default())
}

#[no_mangle]
pub extern "C" fn dead_coin(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&DEAD_COIN.get().unwrap_or([0u8; 32]))
}

#[no_mangle]
pub extern "C" fn resurgence_token(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&RESURGENCE_TOKEN.get().unwrap_or([0u8; 32]))
}

#[no_mangle]
pub extern "C" fn reward_distributor(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&REWARD_DISTRIBUTOR.get().unwrap_or([0u8; 32]))
}

#[no_mangle]
pub extern "C" fn staking_pool_manager(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&STAKING_POOL_MANAGER.get().unwrap_or([0u8; 32]))
}

#[no_mangle]
pub extern "C" fn last_update_time(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&LAST_UPDATE_TIME.get_or_default())
}

#[no_mangle]
pub extern "C" fn reward_per_token_stored(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&REWARD_PER_TOKEN_STORED.get_or_default())
}

#[no_mangle]
pub extern "C" fn user_reward_per_token_paid(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    return_data(&USER_REWARD_PER_TOKEN_PAID.get_or_default(&user))
}

#[no_mangle]
pub extern "C" fn user_rewards(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    return_data(&USER_REWARDS.get_or_default(&user))
}
