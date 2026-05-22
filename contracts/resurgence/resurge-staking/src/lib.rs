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
const RESURGENCE_TOKEN: StorageValue<Address> = StorageValue::new(b"resurgence_token");
const REWARD_DISTRIBUTOR: StorageValue<Address> = StorageValue::new(b"reward_distributor");

const REWARD_RATE_PER_SECOND: StorageValue<u128> = StorageValue::new(b"reward_rate");
const LAST_UPDATE_TIME: StorageValue<u64> = StorageValue::new(b"last_update");
const REWARD_PER_TOKEN_STORED: StorageValue<u128> = StorageValue::new(b"reward_per_token_stored");

const USER_STAKED_AMOUNT: StorageMap<Address, u128> = StorageMap::new(b"user_staked");
const USER_REWARD_PER_TOKEN_PAID: StorageMap<Address, u128> = StorageMap::new(b"user_paid");
const USER_REWARDS: StorageMap<Address, u128> = StorageMap::new(b"user_rewards");
const USER_STAKED_AT: StorageMap<Address, u64> = StorageMap::new(b"user_staked_at");
const USER_DELEGATION: StorageMap<Address, Address> = StorageMap::new(b"user_delegation");

const TOTAL_STAKED_SUPPLY: StorageValue<u128> = StorageValue::new(b"total_staked");
const MIN_STAKE_DURATION: StorageValue<u64> = StorageValue::new(b"min_stake_duration");
const EARLY_UNSTAKE_PENALTY_BPS: StorageValue<u128> = StorageValue::new(b"early_unstake_penalty");

const BASE_BOOST_BPS: StorageValue<u128> = StorageValue::new(b"base_boost_bps");
const USER_BOOST_BPS: StorageMap<Address, u128> = StorageMap::new(b"user_boost_bps");
const PAUSED: StorageValue<bool> = StorageValue::new(b"paused");

fn rate_setter_role() -> [u8; 32] {
    resurgence_common::auth::hash_role("RATE_SETTER_ROLE")
}

#[no_mangle]
pub extern "C" fn initialize(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let token_address: Address = deserialize_arg(&args, 0);
    let distributor_address: Address = deserialize_arg(&args, 1);
    let timelock: Address = deserialize_arg(&args, 2);
    let initial_reward_rate: u128 = deserialize_arg(&args, 3);

    if INITIALIZED.get_or_default() {
        revert("Already initialized");
    }
    if token_address == [0u8; 32] || distributor_address == [0u8; 32] || timelock == [0u8; 32] {
        revert("Invalid addresses");
    }

    INITIALIZED.set(&true);
    RESURGENCE_TOKEN.set(&token_address);
    REWARD_DISTRIBUTOR.set(&distributor_address);
    REWARD_RATE_PER_SECOND.set(&initial_reward_rate);
    LAST_UPDATE_TIME.set(&resurgence_common::host::block_timestamp());

    MIN_STAKE_DURATION.set(&604800); // 7 days in seconds
    EARLY_UNSTAKE_PENALTY_BPS.set(&500); // 5%
    BASE_BOOST_BPS.set(&10000); // 1x

    grant_role(DEFAULT_ADMIN_ROLE, timelock);
    grant_role(timelock_role(), timelock);
    grant_role(emergency_pauser_role(), timelock);
    grant_role(rate_setter_role(), timelock);

    // Grant temporary roles to deployer for setup
    let caller = sender();
    grant_role(DEFAULT_ADMIN_ROLE, caller);
    grant_role(timelock_role(), caller);

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
    let boost = {
        let b = USER_BOOST_BPS.get_or_default(&account);
        if b > 0 {
            b
        } else {
            BASE_BOOST_BPS.get_or_default()
        }
    };
    let staked = USER_STAKED_AMOUNT.get_or_default(&account);
    let paid = USER_REWARD_PER_TOKEN_PAID.get_or_default(&account);
    let current_rpt = reward_per_token();
    let diff = sub(current_rpt, paid);
    let pending_rewards_scaled = mul(staked, diff);
    let pending_rewards = pending_rewards_scaled / 1_000_000_000_000_000_000u128;
    let base = add(USER_REWARDS.get_or_default(&account), pending_rewards);
    mul(base, boost) / 10000
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

fn stake_internal(staker: Address, amount: u128, delegatee: Address, from_address: Address) {
    if amount == 0 {
        revert("Invalid amount");
    }
    let token = RESURGENCE_TOKEN.get().unwrap_or_else(|| revert("Resurgence token not set"));
    let call_args = vec![
        bincode::serialize(&from_address).unwrap(),
        bincode::serialize(&contract_id()).unwrap(),
        bincode::serialize(&amount).unwrap(),
    ];
    let serialized_call_args = bincode::serialize(&call_args).unwrap();
    let res = call_contract(&token, "transfer_from", &serialized_call_args, 0);
    if res < 0 {
        revert("ResurgeStakingPool: transfer_from failed");
    }

    let staked = USER_STAKED_AMOUNT.get_or_default(&staker);
    USER_STAKED_AMOUNT.set(&staker, &add(staked, amount));
    TOTAL_STAKED_SUPPLY.set(&add(TOTAL_STAKED_SUPPLY.get_or_default(), amount));
    USER_STAKED_AT.set(&staker, &resurgence_common::host::block_timestamp());

    if delegatee != [0u8; 32] {
        USER_DELEGATION.set(&staker, &delegatee);
        emit_event(b"DelegationUpdated", &bincode::serialize(&(staker, delegatee)).unwrap());
    }

    emit_event(b"Staked", &bincode::serialize(&(staker, amount, delegatee)).unwrap());
}

#[no_mangle]
pub extern "C" fn stake(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let amount: u128 = deserialize_arg(&args, 0);
    let delegatee: Address = deserialize_arg(&args, 1);

    let caller = sender();
    update_reward(Some(caller));
    stake_internal(caller, amount, delegatee, caller);
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn stake_for(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let args = get_input_args(len);
    let pool_address: Address = deserialize_arg(&args, 0);
    let user: Address = deserialize_arg(&args, 1);
    let amount: u128 = deserialize_arg(&args, 2);

    if sender() != pool_address {
        revert("ResurgeStakingPool: caller/source mismatch");
    }

    update_reward(Some(user));
    stake_internal(user, amount, [0u8; 32], pool_address);
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

    let mut penalty = 0u128;
    let now = resurgence_common::host::block_timestamp();
    let staked_at = USER_STAKED_AT.get_or_default(&caller);
    let duration = MIN_STAKE_DURATION.get_or_default();
    if now < staked_at + duration {
        let penalty_bps = EARLY_UNSTAKE_PENALTY_BPS.get_or_default();
        penalty = mul(amount, penalty_bps) / 10000;
    }

    USER_STAKED_AMOUNT.set(&caller, &sub(staked, amount));
    TOTAL_STAKED_SUPPLY.set(&sub(TOTAL_STAKED_SUPPLY.get_or_default(), amount));

    let return_amount = sub(amount, penalty);
    let token = RESURGENCE_TOKEN.get().unwrap_or_else(|| revert("Resurgence token not set"));

    let call_args1 =
        vec![bincode::serialize(&caller).unwrap(), bincode::serialize(&return_amount).unwrap()];
    let serialized_call_args1 = bincode::serialize(&call_args1).unwrap();
    let res1 = call_contract(&token, "transfer", &serialized_call_args1, 0);
    if res1 < 0 {
        revert("ResurgeStakingPool: transfer failed");
    }

    if penalty > 0 {
        let call_args2 = vec![bincode::serialize(&penalty).unwrap()];
        let serialized_call_args2 = bincode::serialize(&call_args2).unwrap();
        let res2 = call_contract(&token, "burn", &serialized_call_args2, 0);
        if res2 < 0 {
            revert("ResurgeStakingPool: burn penalty failed");
        }
    }

    emit_event(b"Unstaked", &bincode::serialize(&(caller, amount, penalty)).unwrap());
    return_data(&true)
}

fn claim_rewards_internal(user: Address) {
    let rewards = USER_REWARDS.get_or_default(&user);
    if rewards > 0 {
        USER_REWARDS.set(&user, &0);
        let distributor = REWARD_DISTRIBUTOR.get().unwrap_or_else(|| revert("Distributor not set"));
        let call_args = vec![
            bincode::serialize(&user).unwrap(),
            bincode::serialize(&rewards).unwrap(),
        ];
        let serialized_call_args = bincode::serialize(&call_args).unwrap();
        let res = call_contract(&distributor, "mint_and_distribute", &serialized_call_args, 0);
        if res < 0 {
            revert("ResurgeStakingPool: call to mint_and_distribute failed");
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
pub extern "C" fn claim_and_restake(_ptr: i32, len: i32) -> i32 {
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }
    let _args = get_input_args(len);
    let caller = sender();
    update_reward(Some(caller));

    let rewards = USER_REWARDS.get_or_default(&caller);
    if rewards > 0 {
        USER_REWARDS.set(&caller, &0);

        let distributor = REWARD_DISTRIBUTOR.get().unwrap_or_else(|| revert("Distributor not set"));
        let this_address = contract_id();
        let call_args = vec![
            bincode::serialize(&this_address).unwrap(),
            bincode::serialize(&rewards).unwrap(),
        ];
        let serialized_call_args = bincode::serialize(&call_args).unwrap();
        let res = call_contract(&distributor, "mint_and_distribute", &serialized_call_args, 0);
        if res < 0 {
            revert("ResurgeStakingPool: call to mint_and_distribute failed");
        }

        let staked = USER_STAKED_AMOUNT.get_or_default(&caller);
        USER_STAKED_AMOUNT.set(&caller, &add(staked, rewards));
        TOTAL_STAKED_SUPPLY.set(&add(TOTAL_STAKED_SUPPLY.get_or_default(), rewards));
        USER_STAKED_AT.set(&caller, &resurgence_common::host::block_timestamp());

        emit_event(b"RewardsClaimed", &bincode::serialize(&(caller, rewards)).unwrap());
        emit_event(b"Staked", &bincode::serialize(&(caller, rewards, [0u8; 32])).unwrap());
    }
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_delegate(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let delegatee: Address = deserialize_arg(&args, 0);

    let caller = sender();
    USER_DELEGATION.set(&caller, &delegatee);
    emit_event(b"DelegationUpdated", &bincode::serialize(&(caller, delegatee)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_reward_rate(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let new_rate: u128 = deserialize_arg(&args, 0);

    check_role(rate_setter_role(), sender());
    if PAUSED.get_or_default() {
        revert("Pausable: paused");
    }

    update_reward(None);
    REWARD_RATE_PER_SECOND.set(&new_rate);

    emit_event(b"RewardRateUpdated", &bincode::serialize(&new_rate).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn setUserBoost(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let user: Address = deserialize_arg(&args, 0);
    let boost_bps: u128 = deserialize_arg(&args, 1);

    check_role(timelock_role(), sender());
    if boost_bps < 10000 {
        revert("Boost must be >= 1x");
    }
    USER_BOOST_BPS.set(&user, &boost_bps);

    emit_event(b"BoostUpdated", &bincode::serialize(&(user, boost_bps)).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn setMinStakeDuration(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let duration: u64 = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    MIN_STAKE_DURATION.set(&duration);

    emit_event(b"MinStakeDurationUpdated", &bincode::serialize(&duration).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn setEarlyUnstakePenalty(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let penalty_bps: u128 = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    if penalty_bps > 2500 {
        revert("Max 25% penalty");
    }
    EARLY_UNSTAKE_PENALTY_BPS.set(&penalty_bps);

    emit_event(b"EarlyUnstakePenaltyUpdated", &bincode::serialize(&penalty_bps).unwrap());
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
pub extern "C" fn get_voting_power(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let account: Address = deserialize_arg(&args, 0);
    return_data(&USER_STAKED_AMOUNT.get_or_default(&account))
}

#[no_mangle]
pub extern "C" fn earned_query(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let account: Address = deserialize_arg(&args, 0);
    return_data(&earned(account))
}

#[no_mangle]
pub extern "C" fn user_staked_amount(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let account: Address = deserialize_arg(&args, 0);
    return_data(&USER_STAKED_AMOUNT.get_or_default(&account))
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
pub extern "C" fn user_delegation(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let account: Address = deserialize_arg(&args, 0);
    return_data(&USER_DELEGATION.get_or_default(&account))
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn stakeFor(ptr: i32, len: i32) -> i32 {
    stake_for(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn claimRewards(ptr: i32, len: i32) -> i32 {
    claim_rewards(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn claimAndRestake(ptr: i32, len: i32) -> i32 {
    claim_and_restake(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn setDelegate(ptr: i32, len: i32) -> i32 {
    set_delegate(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn setRewardRate(ptr: i32, len: i32) -> i32 {
    set_reward_rate(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn getVotingPower(ptr: i32, len: i32) -> i32 {
    get_voting_power(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn userStakedAmount(ptr: i32, len: i32) -> i32 {
    user_staked_amount(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn totalStakedSupply(ptr: i32, len: i32) -> i32 {
    total_staked_supply(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn rewardRatePerSecond(ptr: i32, len: i32) -> i32 {
    reward_rate_per_second(ptr, len)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn userDelegation(ptr: i32, len: i32) -> i32 {
    user_delegation(ptr, len)
}
