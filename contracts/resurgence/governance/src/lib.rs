use resurgence_common::auth::{check_role, grant_role, timelock_role, DEFAULT_ADMIN_ROLE};
use resurgence_common::host::{
    call_contract, deserialize_arg, emit_event, get_input_args, return_data, revert, sender,
};
use resurgence_common::math::add;
use resurgence_common::storage::{StorageDoubleMap, StorageMap, StorageValue};
use resurgence_common::types::Address;

const INITIALIZED: StorageValue<bool> = StorageValue::new(b"initialized");
const TIMELOCK_DELAY: StorageValue<u64> = StorageValue::new(b"timelock_delay");
const PROPOSALS_COUNT: StorageValue<u64> = StorageValue::new(b"proposals_count");
const QUORUM_VOTES: StorageValue<u128> = StorageValue::new(b"quorum_votes");

const PROPOSALS: StorageMap<u64, Proposal> = StorageMap::new(b"proposals");
const VOTED: StorageDoubleMap<u64, Address, bool> = StorageDoubleMap::new(b"voted");

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Proposal {
    pub id: u64,
    pub creator: Address,
    pub targets: Vec<Address>,
    pub methods: Vec<String>,
    pub args: Vec<Vec<u8>>,
    pub values: Vec<u64>,
    pub description: String,
    pub start_time: u64,
    pub end_time: u64,
    pub for_votes: u128,
    pub against_votes: u128,
    pub abstain_votes: u128,
    pub eta: u64,
    pub executed: bool,
    pub canceled: bool,
}

#[no_mangle]
pub extern "C" fn initialize(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let timelock: Address = deserialize_arg(&args, 0);
    let delay: u64 = deserialize_arg(&args, 1);
    let initial_quorum: u128 = deserialize_arg(&args, 2);

    if INITIALIZED.get_or_default() {
        revert("Already initialized");
    }
    if timelock == [0u8; 32] {
        revert("Invalid timelock address");
    }

    INITIALIZED.set(&true);
    TIMELOCK_DELAY.set(&delay);
    QUORUM_VOTES.set(&initial_quorum);

    grant_role(DEFAULT_ADMIN_ROLE, timelock);
    grant_role(timelock_role(), timelock);

    let caller = sender();
    grant_role(DEFAULT_ADMIN_ROLE, caller);
    grant_role(timelock_role(), caller);

    return_data(&true)
}

#[no_mangle]
pub extern "C" fn propose(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let targets: Vec<Address> = deserialize_arg(&args, 0);
    let methods: Vec<String> = deserialize_arg(&args, 1);
    let call_args: Vec<Vec<u8>> = deserialize_arg(&args, 2);
    let values: Vec<u64> = deserialize_arg(&args, 3);
    let description: String = deserialize_arg(&args, 4);
    let voting_delay: u64 = deserialize_arg(&args, 5);
    let voting_period: u64 = deserialize_arg(&args, 6);

    if targets.len() != methods.len()
        || targets.len() != call_args.len()
        || targets.len() != values.len()
    {
        revert("Mismatched action arrays");
    }

    let caller = sender();
    let proposal_id = PROPOSALS_COUNT
        .get_or_default()
        .checked_add(1)
        .unwrap_or_else(|| revert("Proposal ID overflow"));
    PROPOSALS_COUNT.set(&proposal_id);

    let now = resurgence_common::host::block_timestamp();
    let start_time = now + voting_delay;
    let end_time = start_time + voting_period;

    let proposal = Proposal {
        id: proposal_id,
        creator: caller,
        targets,
        methods,
        args: call_args,
        values,
        description: description.clone(),
        start_time,
        end_time,
        for_votes: 0,
        against_votes: 0,
        abstain_votes: 0,
        eta: 0,
        executed: false,
        canceled: false,
    };

    PROPOSALS.set(&proposal_id, &proposal);
    emit_event(
        b"ProposalCreated",
        &bincode::serialize(&(proposal_id, caller, description)).unwrap(),
    );
    return_data(&proposal_id)
}

#[no_mangle]
pub extern "C" fn cast_vote(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let proposal_id: u64 = deserialize_arg(&args, 0);
    let support: u8 = deserialize_arg(&args, 1);
    let voting_power: u128 = deserialize_arg(&args, 2);

    let mut proposal = PROPOSALS.get(&proposal_id).unwrap_or_else(|| revert("Proposal not found"));
    if proposal.canceled || proposal.executed {
        revert("Proposal not active");
    }

    let now = resurgence_common::host::block_timestamp();
    if now < proposal.start_time || now > proposal.end_time {
        revert("Voting window closed");
    }

    let voter = sender();
    if VOTED.get_or_default(&proposal_id, &voter) {
        revert("Already voted");
    }

    VOTED.set(&proposal_id, &voter, &true);

    match support {
        0 => proposal.against_votes = add(proposal.against_votes, voting_power),
        1 => proposal.for_votes = add(proposal.for_votes, voting_power),
        2 => proposal.abstain_votes = add(proposal.abstain_votes, voting_power),
        _ => revert("Invalid support value"),
    }

    PROPOSALS.set(&proposal_id, &proposal);
    emit_event(
        b"VoteCast",
        &bincode::serialize(&(proposal_id, voter, support, voting_power)).unwrap(),
    );
    return_data(&voting_power)
}

#[no_mangle]
pub extern "C" fn queue(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let proposal_id: u64 = deserialize_arg(&args, 0);

    let mut proposal = PROPOSALS.get(&proposal_id).unwrap_or_else(|| revert("Proposal not found"));
    if proposal.canceled || proposal.executed || proposal.eta > 0 {
        revert("Invalid proposal state");
    }

    let now = resurgence_common::host::block_timestamp();
    if now <= proposal.end_time {
        revert("Voting period not ended");
    }

    let total_votes = add(add(proposal.for_votes, proposal.against_votes), proposal.abstain_votes);
    if total_votes < QUORUM_VOTES.get_or_default() {
        revert("Quorum not met");
    }

    if proposal.for_votes <= proposal.against_votes {
        revert("Proposal defeated");
    }

    let eta = now + TIMELOCK_DELAY.get_or_default();
    proposal.eta = eta;
    PROPOSALS.set(&proposal_id, &proposal);

    emit_event(b"ProposalQueued", &bincode::serialize(&(proposal_id, eta)).unwrap());
    return_data(&eta)
}

#[no_mangle]
pub extern "C" fn execute(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let proposal_id: u64 = deserialize_arg(&args, 0);

    let mut proposal = PROPOSALS.get(&proposal_id).unwrap_or_else(|| revert("Proposal not found"));
    if proposal.canceled || proposal.executed || proposal.eta == 0 {
        revert("Proposal not queued");
    }

    let now = resurgence_common::host::block_timestamp();
    if now < proposal.eta {
        revert("Timelock delay active");
    }

    proposal.executed = true;
    PROPOSALS.set(&proposal_id, &proposal);

    for i in 0..proposal.targets.len() {
        let target = proposal.targets[i];
        let method = &proposal.methods[i];
        let arg = &proposal.args[i];
        let value = proposal.values[i];

        let res = call_contract(&target, method, arg, value);
        if res < 0 {
            revert("Execution of action failed");
        }
    }

    emit_event(b"ProposalExecuted", &bincode::serialize(&proposal_id).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn cancel(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let proposal_id: u64 = deserialize_arg(&args, 0);

    let mut proposal = PROPOSALS.get(&proposal_id).unwrap_or_else(|| revert("Proposal not found"));
    if proposal.executed || proposal.canceled {
        revert("Already finished");
    }

    let caller = sender();
    if caller != proposal.creator && !resurgence_common::auth::has_role(timelock_role(), caller) {
        revert("Not authorized to cancel");
    }

    proposal.canceled = true;
    PROPOSALS.set(&proposal_id, &proposal);

    emit_event(b"ProposalCanceled", &bincode::serialize(&proposal_id).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_quorum(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let new_quorum: u128 = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    QUORUM_VOTES.set(&new_quorum);
    emit_event(b"QuorumUpdated", &bincode::serialize(&new_quorum).unwrap());
    return_data(&true)
}

#[no_mangle]
pub extern "C" fn set_timelock_delay(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let new_delay: u64 = deserialize_arg(&args, 0);

    check_role(timelock_role(), sender());
    TIMELOCK_DELAY.set(&new_delay);
    emit_event(b"TimelockDelayUpdated", &bincode::serialize(&new_delay).unwrap());
    return_data(&true)
}

// Queries
#[no_mangle]
pub extern "C" fn get_proposal(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let proposal_id: u64 = deserialize_arg(&args, 0);
    let prop = PROPOSALS.get(&proposal_id).unwrap_or_else(|| revert("Proposal not found"));
    return_data(&prop)
}

#[no_mangle]
pub extern "C" fn proposals_count(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&PROPOSALS_COUNT.get_or_default())
}

#[no_mangle]
pub extern "C" fn quorum_votes(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&QUORUM_VOTES.get_or_default())
}

#[no_mangle]
pub extern "C" fn timelock_delay(_ptr: i32, len: i32) -> i32 {
    let _args = get_input_args(len);
    return_data(&TIMELOCK_DELAY.get_or_default())
}

#[no_mangle]
pub extern "C" fn has_voted(_ptr: i32, len: i32) -> i32 {
    let args = get_input_args(len);
    let proposal_id: u64 = deserialize_arg(&args, 0);
    let voter: Address = deserialize_arg(&args, 1);
    return_data(&VOTED.get_or_default(&proposal_id, &voter))
}
