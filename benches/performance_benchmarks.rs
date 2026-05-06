use baals::*;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Instant;
use tempfile::TempDir;

fn setup_runtime() -> Runtime<SledStorage, PoAConsensus, NoopSync> {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().to_path_buf();

    let storage = SledStorage::new(&data_dir).unwrap();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
    let test_key = PublicKey::from(signing_key.verifying_key());
    let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();
    let consensus = PoAConsensus::new(test_key, 1000).with_signing_key(signing_key);
    let sync_layer = NoopSync;

    let runtime = Runtime::with_mempool_limit(
        storage.clone(),
        consensus,
        contract_engine,
        sync_layer,
        100000,
    )
    .unwrap();

    runtime.start().unwrap();
    runtime
}

fn create_test_transaction(
    sender: &PublicKey,
    signing_key: &ed25519_dalek::SigningKey,
    nonce: u64,
) -> Transaction {
    let mut tx = Transaction {
        hash: [0u8; 32],
        sender: *sender,
        recipient: Address::Wallet(*sender),
        payload: TransactionPayload::Transfer { amount: 1 },
        nonce,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
        gas_limit: 100000,
        priority: 0,
        metadata: None,
    };
    tx.hash = tx.calculate_hash().unwrap();
    tx.sign(signing_key).unwrap();
    tx
}

fn benchmark_transaction_submission(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_submission");

    group.bench_function("single_transaction", |b| {
        let runtime = setup_runtime();
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet {
            balance: 1000000,
            nonce: 0,
        };
        runtime.create_account(&public_key, account).unwrap();

        b.iter(|| {
            let tx = create_test_transaction(&public_key, &signing_key, black_box(1));
            runtime.submit_transaction(tx).unwrap();
        });
    });

    group.bench_function("batch_transactions", |b| {
        let runtime = setup_runtime();
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet {
            balance: 1000000,
            nonce: 0,
        };
        runtime.create_account(&public_key, account).unwrap();

        b.iter(|| {
            for i in 1..=100 {
                let tx = create_test_transaction(&public_key, &signing_key, i);
                runtime.submit_transaction(tx).unwrap();
            }
        });
    });

    group.finish();
}

fn benchmark_block_production(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_production");

    group.bench_function("empty_block", |b| {
        let runtime = setup_runtime();
        let tokio_runtime = tokio::runtime::Runtime::new().unwrap();

        b.iter(|| {
            let block = tokio_runtime.block_on(async { runtime.produce_block().await.unwrap() });
            black_box(block);
        });
    });

    group.bench_function("block_with_transactions", |b| {
        let runtime = setup_runtime();
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet {
            balance: 1000000,
            nonce: 0,
        };
        runtime.create_account(&public_key, account).unwrap();

        // Pre-submit transactions
        for i in 1..=100 {
            let tx = create_test_transaction(&public_key, &signing_key, i);
            runtime.submit_transaction(tx).unwrap();
        }

        let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            let block = tokio_runtime.block_on(async { runtime.produce_block().await.unwrap() });
            black_box(block);
        });
    });

    group.finish();
}

fn benchmark_storage_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_operations");

    group.bench_function("block_storage", |b| {
        let temp_dir = TempDir::new().unwrap();
        let storage = SledStorage::new(temp_dir.path()).unwrap();

        let test_block = Block {
            index: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            prev_hash: [0u8; 32],
            hash: [1u8; 32],
            transactions: vec![],
            nonce: 0,
            metadata: None,
        };

        b.iter(|| {
            storage.put_block(&test_block).unwrap();
            let retrieved = storage.get_block(&test_block.hash).unwrap().unwrap();
            black_box(retrieved);
        });
    });

    group.bench_function("account_storage", |b| {
        let temp_dir = TempDir::new().unwrap();
        let storage = SledStorage::new(temp_dir.path()).unwrap();

        let test_key = PublicKey::from(ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]).verifying_key());
        let test_account = Account::Wallet {
            balance: 1000,
            nonce: 0,
        };

        b.iter(|| {
            storage.put_account(&test_key, &test_account).unwrap();
            let retrieved = storage.get_account(&test_key).unwrap().unwrap();
            black_box(retrieved);
        });
    });

    group.bench_function("transaction_indexing", |b| {
        let temp_dir = TempDir::new().unwrap();
        let storage = SledStorage::new(temp_dir.path()).unwrap();

        let test_tx = Transaction {
            hash: [1u8; 32],
            sender: PublicKey::from(ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]).verifying_key()),
            recipient: Address::Wallet(PublicKey::from(ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]).verifying_key())),
            payload: TransactionPayload::Transfer { amount: 100 },
            nonce: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            priority: 0,
            metadata: None,
        };

        b.iter(|| {
            storage.put_transaction(&test_tx).unwrap();
            storage
                .index_transaction(&test_tx.hash, &[2u8; 32], 0)
                .unwrap();
            let retrieved = storage.get_transaction(&test_tx.hash).unwrap().unwrap();
            black_box(retrieved);
        });
    });

    group.finish();
}

fn benchmark_contract_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("contract_operations");

    group.bench_function("contract_deployment", |b| {
        let temp_dir = TempDir::new().unwrap();
        let storage = SledStorage::new(temp_dir.path()).unwrap();
        let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();

        let wasm_bytes = create_test_wasm_module();
        let deployer = PublicKey::from(ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]).verifying_key());

        b.iter(|| {
            let contract_id = contract_engine
                .deploy_contract(&deployer, &wasm_bytes, None, &storage, 100000)
                .unwrap();
            black_box(contract_id);
        });
    });

    group.bench_function("contract_execution", |b| {
        let temp_dir = TempDir::new().unwrap();
        let storage = SledStorage::new(temp_dir.path()).unwrap();
        let contract_engine = BaaLSContractEngine::new(storage.clone()).unwrap();

        let contract_id = ContractId::from_bytes(&[1u8; 32]).unwrap();
        let caller = PublicKey::from(ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]).verifying_key());

        b.iter(|| {
            let result = contract_engine
                .call_contract(&caller, &contract_id, "test_method", &[], &storage)
                .unwrap();
            black_box(result);
        });
    });

    group.finish();
}

fn benchmark_cryptographic_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("cryptographic_operations");

    group.bench_function("transaction_signing", |b| {
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());

        let mut tx = Transaction {
            hash: [0u8; 32],
            sender: public_key,
            recipient: Address::Wallet(public_key),
            payload: TransactionPayload::Transfer { amount: 100 },
            nonce: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            signature: TransactionSignature::from_bytes(&[0u8; 64]).unwrap(),
            gas_limit: 100000,
            priority: 0,
            metadata: None,
        };

        b.iter(|| {
            tx.hash = tx.calculate_hash().unwrap();
            tx.sign(&signing_key).unwrap();
            black_box(&tx);
        });
    });

    group.bench_function("hash_calculation", |b| {
        let data = vec![1u8; 1024];

        b.iter(|| {
            let hash = sha2::Sha256::digest(&data);
            black_box(hash);
        });
    });

    group.finish();
}

fn benchmark_mempool_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("mempool_operations");

    group.bench_function("mempool_insertion", |b| {
        let runtime = setup_runtime();
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet {
            balance: 1000000,
            nonce: 0,
        };
        runtime.create_account(&public_key, account).unwrap();

        b.iter(|| {
            for i in 1..=10 {
                let tx = create_test_transaction(&public_key, &signing_key, i);
                runtime.submit_transaction(tx).unwrap();
            }
        });
    });

    group.bench_function("mempool_retrieval", |b| {
        let runtime = setup_runtime();
        let signing_key =
            Runtime::<SledStorage, PoAConsensus, NoopSync>::generate_signing_key().unwrap();
        let public_key = PublicKey::from(signing_key.verifying_key());
        let account = Account::Wallet {
            balance: 1000000,
            nonce: 0,
        };
        runtime.create_account(&public_key, account).unwrap();

        // Pre-populate mempool
        for i in 1..=100 {
            let tx = create_test_transaction(&public_key, &signing_key, i);
            runtime.submit_transaction(tx).unwrap();
        }

        b.iter(|| {
            let pending_txs = runtime.get_pending_transactions().unwrap();
            black_box(pending_txs);
        });
    });

    group.finish();
}

fn benchmark_consensus_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("consensus_operations");

    group.bench_function("block_validation", |b| {
        let test_key = PublicKey::from(ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]).verifying_key());
        let consensus = PoAConsensus::new(test_key, 1000);

        let block = Block {
            index: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            prev_hash: [0u8; 32],
            hash: [1u8; 32],
            transactions: vec![],
            nonce: 0,
            metadata: None,
        };

        b.iter(|| {
            let result = consensus.validate_block(&block);
            black_box(result);
        });
    });

    group.finish();
}

fn create_test_wasm_module() -> Vec<u8> {
    vec![
        0x00, 0x61, 0x73, 0x6d, // WASM magic number
        0x01, 0x00, 0x00, 0x00, // WASM version
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // Type section
        0x03, 0x02, 0x01, 0x00, // Function section
        0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b, // Code section
    ]
}

criterion_group!(
    benches,
    benchmark_transaction_submission,
    benchmark_block_production,
    benchmark_storage_operations,
    benchmark_contract_operations,
    benchmark_cryptographic_operations,
    benchmark_mempool_operations,
    benchmark_consensus_operations,
);
criterion_main!(benches);
