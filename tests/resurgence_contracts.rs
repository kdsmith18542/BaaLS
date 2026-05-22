use baals::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Proposal {
    pub id: u64,
    pub creator: [u8; 32],
    pub targets: Vec<[u8; 32]>,
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

fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .try_init();
}

fn make_runtime(
    data_dir: &std::path::Path,
) -> (
    Runtime<SledStorage, PoAConsensus, NoopSync>,
    ed25519_dalek::SigningKey,
    PublicKey,
) {
    let storage = SledStorage::new(data_dir).unwrap();
    let consensus_signing_key =
        Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let test_key = PublicKey::from(consensus_signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus =
        PoAConsensus::new(test_key, 1000).with_signing_key(consensus_signing_key.clone());
    let sync_layer = NoopSync;
    let mut runtime =
        Runtime::with_mempool_limit(storage.clone(), consensus, contract_engine, sync_layer, 1000)
            .unwrap();
    runtime.produce_empty_blocks = true;
    runtime.start().unwrap();
    (runtime, consensus_signing_key, test_key)
}

fn make_test_account(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    balance: u64,
) -> (ed25519_dalek::SigningKey, PublicKey) {
    let sk = Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
    let pk = PublicKey::from(sk.verifying_key());
    runtime.create_account(&pk, Account::Wallet { balance, nonce: 0 }).unwrap();
    (sk, pk)
}

#[allow(clippy::too_many_arguments)]
fn make_contract_call_tx(
    sender_pk: PublicKey,
    sender_sk: &ed25519_dalek::SigningKey,
    contract_id: &ContractId,
    method: &str,
    args: Vec<Vec<u8>>,
    value: Option<u64>,
    nonce: u64,
    gas_limit: u64,
    gas_price: u64,
    chain_id: u64,
) -> Transaction {
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: sender_pk,
        recipient: Address::Contract(contract_id.clone()),
        payload: TransactionPayload::ContractCall { method: method.to_string(), args, value },
        nonce,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit,
        gas_price,
        priority: 0,
        metadata: None,
        chain_id,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(sender_sk).unwrap();
    tx
}

fn load_wasm(name: &str) -> Vec<u8> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("contracts");
    path.push("resurgence");
    path.push("target");
    path.push("wasm32-unknown-unknown");
    path.push("release");
    path.push(format!("{}.wasm", name));
    std::fs::read(&path).unwrap_or_else(|e| panic!("Failed to read WASM file at {:?}: {}", path, e))
}

fn resurgence_workspace_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("contracts");
    path.push("resurgence");
    path
}

fn ensure_resurgence_wasm_built() {
    let workspace_dir = resurgence_workspace_dir();
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--workspace"])
        .current_dir(&workspace_dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "Failed to invoke cargo build for resurgence WASM contracts in {:?}: {}",
                workspace_dir, e
            )
        });

    assert!(
        status.success(),
        "Failed to build resurgence WASM contracts in {:?}",
        workspace_dir
    );
}

fn deploy_contract_helper(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    deployer_pk: &PublicKey,
    wasm_bytes: &[u8],
    init_payload: Option<&[u8]>,
) -> ContractId {
    let account = runtime.get_account(deployer_pk).unwrap().unwrap();
    let current_nonce = account.nonce();
    let balance = account.balance();

    let contract_id =
        runtime.deploy_contract(deployer_pk, wasm_bytes, init_payload, 2_000_000).unwrap();

    // Explicitly update the nonce of the deployer so next deploy doesn't collide
    runtime
        .create_account(deployer_pk, Account::Wallet { balance, nonce: current_nonce + 1 })
        .unwrap();
    contract_id
}

fn submit_tx_and_produce_block(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    tx: Transaction,
) {
    let tx_hash = tx.hash;
    runtime.submit_transaction(tx).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    let receipt = runtime.storage().get_receipt(&tx_hash).unwrap().expect("Receipt not found");
    assert!(receipt.success, "Transaction failed: {:?}", receipt.error_message);
}

fn submit_tx_and_expect_failure(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    tx: Transaction,
) {
    let tx_hash = tx.hash;
    runtime.submit_transaction(tx).unwrap();

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    let receipt = runtime.storage().get_receipt(&tx_hash).unwrap().expect("Receipt not found");
    assert!(
        !receipt.success,
        "Transaction unexpectedly succeeded when failure was expected"
    );
}

fn submit_contract_call_tx(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    sender_pk: PublicKey,
    sender_sk: &ed25519_dalek::SigningKey,
    contract_id: &ContractId,
    method: &str,
    args: Vec<Vec<u8>>,
) {
    let account = runtime.get_account(&sender_pk).unwrap().unwrap();
    let next_nonce = account.nonce() + 1;
    let tx = make_contract_call_tx(
        sender_pk,
        sender_sk,
        contract_id,
        method,
        args,
        None,
        next_nonce,
        2_000_000, // gas limit
        1,         // gas price
        1,         // chain ID
    );
    submit_tx_and_produce_block(runtime, tx);
}

fn submit_contract_call_tx_expect_failure(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    sender_pk: PublicKey,
    sender_sk: &ed25519_dalek::SigningKey,
    contract_id: &ContractId,
    method: &str,
    args: Vec<Vec<u8>>,
) {
    let account = runtime.get_account(&sender_pk).unwrap().unwrap();
    let next_nonce = account.nonce() + 1;
    let tx = make_contract_call_tx(
        sender_pk,
        sender_sk,
        contract_id,
        method,
        args,
        None,
        next_nonce,
        2_000_000,
        1,
        1,
    );
    submit_tx_and_expect_failure(runtime, tx);
}

fn query_contract<T: serde::de::DeserializeOwned>(
    runtime: &Runtime<SledStorage, PoAConsensus, NoopSync>,
    caller_pk: &PublicKey,
    contract_id: &ContractId,
    method: &str,
    args: Vec<Vec<u8>>,
) -> T {
    let res =
        runtime.call_contract(caller_pk, contract_id, method, &args, None, 2_000_000).unwrap();
    bincode::deserialize(&res).unwrap_or_else(|e| {
        panic!("Failed to deserialize return value of query '{}': {}", method, e)
    })
}

pub fn hash_role(role_name: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(role_name.as_bytes());
    hasher.finalize().into()
}

pub fn minter_role() -> [u8; 32] {
    hash_role("MINTER_ROLE")
}

pub fn timelock_role_hash() -> [u8; 32] {
    hash_role("TIMELOCK_ROLE")
}

#[test]
fn test_resurgence_staking_and_governance() {
    init_logging();
    ensure_resurgence_wasm_built();
    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());

    // Create admin and user accounts with sufficient balances
    let (admin_sk, admin_pk) = make_test_account(&runtime, 10_000_000);
    let (user_sk, user_pk) = make_test_account(&runtime, 10_000_000);

    // 1. Deploy contracts
    println!("[TEST] Deploying contracts...");

    // Deploy RESURGE token contract
    let token_wasm = load_wasm("resurge_token");
    let resurge_token_id = deploy_contract_helper(&runtime, &admin_pk, &token_wasm, None);

    // Initialize RESURGE token contract
    let resurge_init_args = vec![
        bincode::serialize(&admin_pk.to_bytes()).unwrap(),
        bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(), // Cap = 1,000,000 * 10^18
    ];
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &resurge_token_id,
        "initialize",
        resurge_init_args,
    );
    println!("[TEST] RESURGE Token deployed and initialized.");

    // Deploy DEADCOIN token contract
    let deadcoin_wasm = load_wasm("resurge_token");
    let deadcoin_id = deploy_contract_helper(&runtime, &admin_pk, &deadcoin_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &deadcoin_id,
        "initialize",
        vec![
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
        ],
    );
    println!("[TEST] DEADCOIN Token deployed and initialized.");

    // Deploy RewardDistributor contract
    let distributor_wasm = load_wasm("reward_distributor");
    let distributor_id = deploy_contract_helper(&runtime, &admin_pk, &distributor_wasm, None);
    let distributor_init_args = vec![
        bincode::serialize(&resurge_token_id.to_bytes()).unwrap(),
        bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
        bincode::serialize(&admin_pk.to_bytes()).unwrap(),
    ];
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &distributor_id,
        "initialize",
        distributor_init_args,
    );
    println!("[TEST] RewardDistributor deployed and initialized.");

    // Deploy StakingPoolManager contract
    let manager_wasm = load_wasm("resurgence_staking_pool_manager");
    let manager_id = deploy_contract_helper(&runtime, &admin_pk, &manager_wasm, None);
    let manager_init_args = vec![
        bincode::serialize(&resurge_token_id.to_bytes()).unwrap(),
        bincode::serialize(&distributor_id.to_bytes()).unwrap(),
        bincode::serialize(&admin_pk.to_bytes()).unwrap(),
    ];
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "initialize",
        manager_init_args,
    );
    println!("[TEST] StakingPoolManager deployed and initialized.");
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &distributor_id,
        "grant_role_entry",
        vec![
            bincode::serialize(&timelock_role_hash()).unwrap(),
            bincode::serialize(&manager_id.to_bytes()).unwrap(),
        ],
    );

    // Deploy governance now so the staking pool can trust it as timelock role holder.
    let gov_wasm = load_wasm("resurgence_governance");
    let gov_id = deploy_contract_helper(&runtime, &admin_pk, &gov_wasm, None);

    // Deploy StakingPool contract
    let pool_wasm = load_wasm("staking_pool");
    let pool_id = deploy_contract_helper(&runtime, &admin_pk, &pool_wasm, None);
    let pool_init_args = vec![
        bincode::serialize(&deadcoin_id.to_bytes()).unwrap(),
        bincode::serialize(&resurge_token_id.to_bytes()).unwrap(),
        bincode::serialize(&distributor_id.to_bytes()).unwrap(),
        bincode::serialize(&manager_id.to_bytes()).unwrap(),
        bincode::serialize(&gov_id.to_bytes()).unwrap(),
    ];
    submit_contract_call_tx(&runtime, admin_pk, &admin_sk, &pool_id, "initialize", pool_init_args);
    println!("[TEST] StakingPool deployed and initialized.");

    // 2. Configure permissions
    println!("[TEST] Granting roles...");
    // Grant minter role to RewardDistributor so it can mint claim rewards.
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &resurge_token_id,
        "grant_role_entry",
        vec![
            bincode::serialize(&minter_role()).unwrap(),
            bincode::serialize(&distributor_id.to_bytes()).unwrap(),
        ],
    );

    // 3. Register the pool on StakingPoolManager
    println!("[TEST] Registering Staking Pool...");
    let register_args = vec![
        bincode::serialize(&deadcoin_id.to_bytes()).unwrap(),
        bincode::serialize(&pool_id.to_bytes()).unwrap(),
        bincode::serialize(&1_000_000_000_000_000_000u128).unwrap(), // initial rate = 1 RESURGE / sec
    ];
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "add_staking_pool",
        register_args,
    );

    // 4. Mint DEADCOIN tokens to user
    println!("[TEST] Minting DEADCOIN to user...");
    let mint_args = vec![
        bincode::serialize(&user_pk.to_bytes()).unwrap(),
        bincode::serialize(&1_000_000_000_000_000_000_000u128).unwrap(), // 1000 * 10^18 DEADCOIN
    ];
    submit_contract_call_tx(&runtime, admin_pk, &admin_sk, &deadcoin_id, "mint", mint_args);

    let user_deadcoin_bal = query_contract::<u128>(
        &runtime,
        &user_pk,
        &deadcoin_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(user_deadcoin_bal, 1_000_000_000_000_000_000_000u128);

    // 5. User approves and stakes DEADCOIN
    println!("[TEST] User staking DEADCOIN...");
    let approve_args = vec![
        bincode::serialize(&pool_id.to_bytes()).unwrap(),
        bincode::serialize(&500_000_000_000_000_000_000u128).unwrap(), // 500 * 10^18 DEADCOIN
    ];
    submit_contract_call_tx(&runtime, user_pk, &user_sk, &deadcoin_id, "approve", approve_args);

    let stake_args = vec![bincode::serialize(&500_000_000_000_000_000_000u128).unwrap()];
    submit_contract_call_tx(&runtime, user_pk, &user_sk, &pool_id, "stake", stake_args);

    // Verify staked amounts
    let user_staked = query_contract::<u128>(
        &runtime,
        &user_pk,
        &pool_id,
        "user_staked_amount",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(user_staked, 500_000_000_000_000_000_000u128);

    let total_staked =
        query_contract::<u128>(&runtime, &user_pk, &pool_id, "total_staked_supply", vec![]);
    assert_eq!(total_staked, 500_000_000_000_000_000_000u128);

    // Debug print before sleep
    let rate_before =
        query_contract::<u128>(&runtime, &user_pk, &pool_id, "reward_rate_per_second", vec![]);
    let last_update_before =
        query_contract::<u64>(&runtime, &user_pk, &pool_id, "last_update_time", vec![]);
    let stored_before =
        query_contract::<u128>(&runtime, &user_pk, &pool_id, "reward_per_token_stored", vec![]);
    println!(
        "[TEST DEBUG] BEFORE SLEEP: rate={}, last_update={}, stored={}",
        rate_before, last_update_before, stored_before
    );

    // 6. Simulate block time progression to accumulate yield
    println!("[TEST] Sleeping to accumulate yield...");
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Produce an empty block to advance the timestamp
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Debug print after sleep and block production
    let rate_after =
        query_contract::<u128>(&runtime, &user_pk, &pool_id, "reward_rate_per_second", vec![]);
    let last_update_after =
        query_contract::<u64>(&runtime, &user_pk, &pool_id, "last_update_time", vec![]);
    let stored_after =
        query_contract::<u128>(&runtime, &user_pk, &pool_id, "reward_per_token_stored", vec![]);
    println!(
        "[TEST DEBUG] AFTER SLEEP & BLOCK: rate={}, last_update={}, stored={}",
        rate_after, last_update_after, stored_after
    );

    // Check earned rewards
    let earned_rewards = query_contract::<u128>(
        &runtime,
        &user_pk,
        &pool_id,
        "earned_query",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    println!("[TEST] Earned rewards after time advancement: {}", earned_rewards);
    assert!(earned_rewards > 0, "Earned rewards should be greater than 0");

    // 7. Claim rewards
    println!("[TEST] Claiming rewards...");
    submit_contract_call_tx(&runtime, user_pk, &user_sk, &pool_id, "claim_rewards", vec![]);

    // Verify user balance of RESURGE is greater than 0
    let user_resurge_bal = query_contract::<u128>(
        &runtime,
        &user_pk,
        &resurge_token_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    println!("[TEST] User RESURGE balance after claim: {}", user_resurge_bal);
    assert!(
        user_resurge_bal > 0,
        "User RESURGE balance should be greater than 0 after claim"
    );

    // 8. Governance Flow Verification
    println!("[TEST] Starting Governance Flow...");
    let gov_init_args = vec![
        bincode::serialize(&admin_pk.to_bytes()).unwrap(), // timelock = admin_pk
        bincode::serialize(&2u64).unwrap(),                // delay = 2 seconds
        bincode::serialize(&100_000_000_000_000_000_000u128).unwrap(), // initial quorum = 100 * 10^18
    ];
    submit_contract_call_tx(&runtime, admin_pk, &admin_sk, &gov_id, "initialize", gov_init_args);
    println!("[TEST] Governance deployed and initialized.");

    // Propose a rate change on pool from 1 * 10^18 to 5 * 10^18
    let set_rate_args = vec![bincode::serialize(&5_000_000_000_000_000_000u128).unwrap()];
    let arg_payload = bincode::serialize(&set_rate_args).unwrap();

    let proposal_args = vec![
        bincode::serialize(&vec![pool_id.to_bytes()]).unwrap(), // targets
        bincode::serialize(&vec!["set_reward_rate".to_string()]).unwrap(), // methods
        bincode::serialize(&vec![arg_payload]).unwrap(),        // call_args
        bincode::serialize(&vec![0u64]).unwrap(),               // values
        bincode::serialize(&"Propose new reward rate".to_string()).unwrap(), // description
        bincode::serialize(&0u64).unwrap(),                     // voting_delay = 0
        bincode::serialize(&2u64).unwrap(),                     // voting_period = 2 seconds
    ];
    submit_contract_call_tx(&runtime, admin_pk, &admin_sk, &gov_id, "propose", proposal_args);

    let count = query_contract::<u64>(&runtime, &admin_pk, &gov_id, "proposals_count", vec![]);
    assert_eq!(count, 1);

    // Vote on proposal
    println!("[TEST] Voting on proposal...");
    let vote_args = vec![
        bincode::serialize(&1u64).unwrap(), // proposal_id
        bincode::serialize(&1u8).unwrap(),  // support = 1 (for)
        bincode::serialize(&200_000_000_000_000_000_000u128).unwrap(), // voting_power = 200 * 10^18
    ];
    submit_contract_call_tx(&runtime, admin_pk, &admin_sk, &gov_id, "cast_vote", vote_args);

    let prop = query_contract::<Proposal>(
        &runtime,
        &admin_pk,
        &gov_id,
        "get_proposal",
        vec![bincode::serialize(&1u64).unwrap()],
    );
    assert_eq!(prop.for_votes, 200_000_000_000_000_000_000u128);

    // Wait for voting period to end (voting_period = 2 seconds)
    println!("[TEST] Waiting for voting period to end...");
    std::thread::sleep(std::time::Duration::from_secs(3));
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Queue proposal
    println!("[TEST] Queueing proposal...");
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &gov_id,
        "queue",
        vec![bincode::serialize(&1u64).unwrap()],
    );

    let prop = query_contract::<Proposal>(
        &runtime,
        &admin_pk,
        &gov_id,
        "get_proposal",
        vec![bincode::serialize(&1u64).unwrap()],
    );
    assert!(prop.eta > 0);

    // Wait for timelock delay to end (timelock_delay = 2 seconds)
    println!("[TEST] Waiting for timelock delay to end...");
    std::thread::sleep(std::time::Duration::from_secs(3));
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    // Execute proposal
    println!("[TEST] Executing proposal...");
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &gov_id,
        "execute",
        vec![bincode::serialize(&1u64).unwrap()],
    );

    let prop = query_contract::<Proposal>(
        &runtime,
        &admin_pk,
        &gov_id,
        "get_proposal",
        vec![bincode::serialize(&1u64).unwrap()],
    );
    assert!(prop.executed);

    // Verify rate change was applied to the staking pool contract
    let new_rate =
        query_contract::<u128>(&runtime, &admin_pk, &pool_id, "reward_rate_per_second", vec![]);
    println!("[TEST] New reward rate on pool contract: {}", new_rate);
    assert_eq!(new_rate, 5_000_000_000_000_000_000u128);

    println!("[TEST] Staking and Governance integration tests passed successfully!");
}

#[test]
fn test_resurge_staking_claim_and_early_unstake_penalty() {
    init_logging();
    ensure_resurgence_wasm_built();

    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (admin_sk, admin_pk) = make_test_account(&runtime, 10_000_000);
    let (user_sk, user_pk) = make_test_account(&runtime, 10_000_000);

    let token_wasm = load_wasm("resurge_token");
    let token_id = deploy_contract_helper(&runtime, &admin_pk, &token_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &token_id,
        "initialize",
        vec![
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
        ],
    );

    let distributor_wasm = load_wasm("reward_distributor");
    let distributor_id = deploy_contract_helper(&runtime, &admin_pk, &distributor_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &distributor_id,
        "initialize",
        vec![
            bincode::serialize(&token_id.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
        ],
    );

    let staking_wasm = load_wasm("resurge_staking");
    let staking_id = deploy_contract_helper(&runtime, &admin_pk, &staking_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &staking_id,
        "initialize",
        vec![
            bincode::serialize(&token_id.to_bytes()).unwrap(),
            bincode::serialize(&distributor_id.to_bytes()).unwrap(),
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000u128).unwrap(), // 1 RESURGE/sec
        ],
    );

    // RewardDistributor mints reward claims via ResurgeToken::mint.
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &token_id,
        "grant_role_entry",
        vec![
            bincode::serialize(&minter_role()).unwrap(),
            bincode::serialize(&distributor_id.to_bytes()).unwrap(),
        ],
    );
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &distributor_id,
        "authorize_pool",
        vec![bincode::serialize(&staking_id.to_bytes()).unwrap()],
    );

    let minted_amount = 1_000_000_000_000_000_000_000u128;
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &token_id,
        "mint",
        vec![
            bincode::serialize(&user_pk.to_bytes()).unwrap(),
            bincode::serialize(&minted_amount).unwrap(),
        ],
    );

    let stake_amount = 200_000_000_000_000_000_000u128;
    submit_contract_call_tx(
        &runtime,
        user_pk,
        &user_sk,
        &token_id,
        "approve",
        vec![
            bincode::serialize(&staking_id.to_bytes()).unwrap(),
            bincode::serialize(&stake_amount).unwrap(),
        ],
    );
    submit_contract_call_tx(
        &runtime,
        user_pk,
        &user_sk,
        &staking_id,
        "stake",
        vec![
            bincode::serialize(&stake_amount).unwrap(),
            bincode::serialize(&user_pk.to_bytes()).unwrap(), // self delegate
        ],
    );

    let user_balance_after_stake = query_contract::<u128>(
        &runtime,
        &user_pk,
        &token_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(user_balance_after_stake, minted_amount - stake_amount);

    let staked_amount = query_contract::<u128>(
        &runtime,
        &user_pk,
        &staking_id,
        "user_staked_amount",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(staked_amount, stake_amount);

    // Advance time to accrue rewards.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    tokio_rt.block_on(runtime.produce_block()).unwrap();

    let earned_rewards = query_contract::<u128>(
        &runtime,
        &user_pk,
        &staking_id,
        "earned_query",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert!(earned_rewards > 0, "Expected accrued rewards after time advancement");

    let user_balance_before_claim = query_contract::<u128>(
        &runtime,
        &user_pk,
        &token_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    submit_contract_call_tx(&runtime, user_pk, &user_sk, &staking_id, "claim_rewards", vec![]);
    let user_balance_after_claim = query_contract::<u128>(
        &runtime,
        &user_pk,
        &token_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert!(user_balance_after_claim > user_balance_before_claim);

    let unstake_amount = 100_000_000_000_000_000_000u128;
    let expected_penalty = unstake_amount * 500 / 10_000; // default 5%
    let expected_return = unstake_amount - expected_penalty;

    let user_balance_before_unstake = query_contract::<u128>(
        &runtime,
        &user_pk,
        &token_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    let total_supply_before_unstake =
        query_contract::<u128>(&runtime, &user_pk, &token_id, "total_supply", vec![]);

    submit_contract_call_tx(
        &runtime,
        user_pk,
        &user_sk,
        &staking_id,
        "unstake",
        vec![bincode::serialize(&unstake_amount).unwrap()],
    );

    let user_balance_after_unstake = query_contract::<u128>(
        &runtime,
        &user_pk,
        &token_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(user_balance_after_unstake - user_balance_before_unstake, expected_return);

    let total_supply_after_unstake =
        query_contract::<u128>(&runtime, &user_pk, &token_id, "total_supply", vec![]);
    assert_eq!(total_supply_before_unstake - total_supply_after_unstake, expected_penalty);

    let remaining_staked = query_contract::<u128>(
        &runtime,
        &user_pk,
        &staking_id,
        "user_staked_amount",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(remaining_staked, stake_amount - unstake_amount);
}

#[test]
fn test_staking_pool_manager_batch_stake_and_dynamic_rate() {
    init_logging();
    ensure_resurgence_wasm_built();

    let temp_dir = TempDir::new().unwrap();
    let (runtime, _consensus_sk, _consensus_pk) = make_runtime(temp_dir.path());
    let (admin_sk, admin_pk) = make_test_account(&runtime, 10_000_000);
    let (user_sk, user_pk) = make_test_account(&runtime, 10_000_000);

    let token_wasm = load_wasm("resurge_token");
    let resurge_token_id = deploy_contract_helper(&runtime, &admin_pk, &token_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &resurge_token_id,
        "initialize",
        vec![
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
        ],
    );

    let deadcoin_id = deploy_contract_helper(&runtime, &admin_pk, &token_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &deadcoin_id,
        "initialize",
        vec![
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
        ],
    );

    let distributor_wasm = load_wasm("reward_distributor");
    let distributor_id = deploy_contract_helper(&runtime, &admin_pk, &distributor_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &distributor_id,
        "initialize",
        vec![
            bincode::serialize(&resurge_token_id.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000_000_000u128).unwrap(),
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
        ],
    );

    let manager_wasm = load_wasm("resurgence_staking_pool_manager");
    let manager_id = deploy_contract_helper(&runtime, &admin_pk, &manager_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "initialize",
        vec![
            bincode::serialize(&resurge_token_id.to_bytes()).unwrap(),
            bincode::serialize(&distributor_id.to_bytes()).unwrap(),
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
        ],
    );
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &distributor_id,
        "grant_role_entry",
        vec![
            bincode::serialize(&timelock_role_hash()).unwrap(),
            bincode::serialize(&manager_id.to_bytes()).unwrap(),
        ],
    );

    let pool_wasm = load_wasm("staking_pool");
    let pool_id = deploy_contract_helper(&runtime, &admin_pk, &pool_wasm, None);
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &pool_id,
        "initialize",
        vec![
            bincode::serialize(&deadcoin_id.to_bytes()).unwrap(),
            bincode::serialize(&resurge_token_id.to_bytes()).unwrap(),
            bincode::serialize(&distributor_id.to_bytes()).unwrap(),
            bincode::serialize(&manager_id.to_bytes()).unwrap(),
            bincode::serialize(&admin_pk.to_bytes()).unwrap(),
        ],
    );

    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "add_staking_pool",
        vec![
            bincode::serialize(&deadcoin_id.to_bytes()).unwrap(),
            bincode::serialize(&pool_id.to_bytes()).unwrap(),
            bincode::serialize(&1_000_000_000_000_000_000u128).unwrap(),
        ],
    );

    // Direct user calls must not be able to mint through RewardDistributor.
    submit_contract_call_tx_expect_failure(
        &runtime,
        user_pk,
        &user_sk,
        &distributor_id,
        "mint_and_distribute",
        vec![
            bincode::serialize(&user_pk.to_bytes()).unwrap(),
            bincode::serialize(&1u128).unwrap(),
        ],
    );

    let mint_amount = 1_000_000_000_000_000_000_000u128;
    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &deadcoin_id,
        "mint",
        vec![
            bincode::serialize(&user_pk.to_bytes()).unwrap(),
            bincode::serialize(&mint_amount).unwrap(),
        ],
    );

    let stake_amount = 200_000_000_000_000_000_000u128;
    submit_contract_call_tx(
        &runtime,
        user_pk,
        &user_sk,
        &deadcoin_id,
        "approve",
        vec![
            bincode::serialize(&manager_id.to_bytes()).unwrap(),
            bincode::serialize(&stake_amount).unwrap(),
        ],
    );
    submit_contract_call_tx(
        &runtime,
        user_pk,
        &user_sk,
        &manager_id,
        "batch_stake",
        vec![
            bincode::serialize(&vec![deadcoin_id.to_bytes()]).unwrap(),
            bincode::serialize(&vec![stake_amount]).unwrap(),
        ],
    );

    let user_staked = query_contract::<u128>(
        &runtime,
        &user_pk,
        &pool_id,
        "user_staked_amount",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(user_staked, stake_amount);

    let user_deadcoin_bal = query_contract::<u128>(
        &runtime,
        &user_pk,
        &deadcoin_id,
        "balance_of",
        vec![bincode::serialize(&user_pk.to_bytes()).unwrap()],
    );
    assert_eq!(user_deadcoin_bal, mint_amount - stake_amount);

    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "set_dynamic_rate_enabled",
        vec![bincode::serialize(&true).unwrap()],
    );

    let tvl = 2_000_000_000_000_000_000_000_000u128; // 2M * 1e18
    let multiplier = 15_000u128; // 1.5x
    let dynamic_rate = query_contract::<u128>(
        &runtime,
        &admin_pk,
        &manager_id,
        "calculate_dynamic_rate",
        vec![bincode::serialize(&tvl).unwrap(), bincode::serialize(&multiplier).unwrap()],
    );
    assert_eq!(dynamic_rate, 1_470_000_000_000_000_000u128);

    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "apply_dynamic_rate",
        vec![
            bincode::serialize(&deadcoin_id.to_bytes()).unwrap(),
            bincode::serialize(&tvl).unwrap(),
            bincode::serialize(&multiplier).unwrap(),
        ],
    );

    let pool_rate =
        query_contract::<u128>(&runtime, &admin_pk, &pool_id, "reward_rate_per_second", vec![]);
    assert_eq!(pool_rate, dynamic_rate);

    submit_contract_call_tx(
        &runtime,
        admin_pk,
        &admin_sk,
        &manager_id,
        "apply_dynamic_rate_all",
        vec![
            bincode::serialize(&vec![deadcoin_id.to_bytes()]).unwrap(),
            bincode::serialize(&vec![tvl]).unwrap(),
            bincode::serialize(&multiplier).unwrap(),
        ],
    );
    let pool_rate_after_all =
        query_contract::<u128>(&runtime, &admin_pk, &pool_id, "reward_rate_per_second", vec![]);
    assert_eq!(pool_rate_after_all, dynamic_rate);
}
