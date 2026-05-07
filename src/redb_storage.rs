use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;
use std::sync::Arc;

use crate::storage::{
    ContractEvents, Storage, StorageBatch, StorageError, StorageOperation, StorageStats,
};
use crate::types::{Account, Block, ChainState, ContractId, PublicKey, Transaction};

const BLOCKS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("blocks");
const TXS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("transactions");
const PENDING_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("pending");
const ACCOUNTS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("accounts");
const CHAIN_STATE_KEY: &[u8] = b"global:current";
const CONTRACTS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("contracts");
const EVENTS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("events");

fn map_err(e: impl std::fmt::Display) -> StorageError {
    StorageError::IndexError(e.to_string())
}

pub struct RedbStorage {
    db: Arc<Database>,
}

impl RedbStorage {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if path.is_dir() {
            std::fs::create_dir_all(path).map_err(map_err)?;
            let db_path = path.join("data.redb");
            let db = Database::create(db_path).map_err(map_err)?;
            Self::init_tables(&db)?;
            Ok(Self { db: Arc::new(db) })
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(map_err)?;
            }
            let db = Database::create(path).map_err(map_err)?;
            Self::init_tables(&db)?;
            Ok(Self { db: Arc::new(db) })
        }
    }

    fn init_tables(db: &Database) -> Result<(), StorageError> {
        let txn = db.begin_write().map_err(map_err)?;
        {
            txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
            txn.open_table(TXS_TABLE).map_err(map_err)?;
            txn.open_table(PENDING_TABLE).map_err(map_err)?;
            txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            txn.open_table(EVENTS_TABLE).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }
}

impl Storage for RedbStorage {
    fn put_block(&self, block: &Block) -> Result<(), StorageError> {
        let val = bincode::serialize(block)?;
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
            table.insert(block.hash.as_slice(), val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
        match table.get(hash.as_slice()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn get_latest_block(&self) -> Result<Option<Block>, StorageError> {
        let chain = self.get_chain_state()?;
        match chain {
            Some(cs) => self.get_block(&cs.latest_block_hash),
            None => Ok(None),
        }
    }

    fn get_chain_height(&self) -> Result<u64, StorageError> {
        let chain = self.get_chain_state()?;
        Ok(chain.map(|cs| cs.latest_block_index).unwrap_or(0))
    }

    fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
        let height_key = format!("height:{}", height);
        match table.get(height_key.as_bytes()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        let val = bincode::serialize(tx)?;
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(TXS_TABLE).map_err(map_err)?;
            table.insert(tx.hash.as_slice(), val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        match table.get(tx_hash.as_slice()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(PENDING_TABLE).map_err(map_err)?;
        let mut txs = Vec::new();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (_, v) = item.map_err(map_err)?;
            if let Ok(tx) = bincode::deserialize::<Transaction>(v.value()) {
                txs.push(tx);
            }
        }
        Ok(txs)
    }

    fn remove_pending_transaction(&self, tx_hash: &[u8; 32]) -> Result<(), StorageError> {
        let key = format!("pending:{}", hex::encode(tx_hash));
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(PENDING_TABLE).map_err(map_err)?;
            table.remove(key.as_bytes()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn put_pending_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        let key = format!("pending:{}", hex::encode(tx.hash));
        let encoded = bincode::serialize(tx)?;
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(PENDING_TABLE).map_err(map_err)?;
            table.insert(key.as_bytes(), encoded.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn index_transaction(
        &self,
        tx_hash: &[u8; 32],
        block_hash: &[u8; 32],
        _tx_index_in_block: u32,
    ) -> Result<(), StorageError> {
        let key = format!("idx:{}:{}", hex::encode(tx_hash), hex::encode(block_hash));
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(TXS_TABLE).map_err(map_err)?;
            table.insert(key.as_bytes(), block_hash.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_transaction_by_id(
        &self,
        tx_hash: &[u8; 32],
    ) -> Result<Option<Transaction>, StorageError> {
        self.get_transaction(tx_hash)
    }

    fn get_transactions_by_block(
        &self,
        _block_hash: &[u8; 32],
    ) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let mut txs = Vec::new();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (_, v) = item.map_err(map_err)?;
            if let Ok(tx) = bincode::deserialize::<Transaction>(v.value()) {
                txs.push(tx);
            }
        }
        Ok(txs)
    }

    fn get_transactions_by_address(
        &self,
        address: &PublicKey,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let mut txs = Vec::new();
        let addr_bytes = address.to_bytes();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (_, v) = item.map_err(map_err)?;
            if let Ok(tx) = bincode::deserialize::<Transaction>(v.value()) {
                if tx.sender.to_bytes() == addr_bytes
                    || matches!(&tx.recipient, crate::types::Address::Wallet(pk) if pk.to_bytes() == addr_bytes)
                {
                    txs.push(tx);
                    if txs.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(txs)
    }

    fn get_transactions_by_height_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let blocks = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
        let txs_table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let mut result = Vec::new();
        let iter = blocks.iter().map_err(map_err)?;
        for item in iter {
            let (_, v) = item.map_err(map_err)?;
            if let Ok(block) = bincode::deserialize::<Block>(v.value()) {
                if block.index >= from && block.index <= to {
                    for tx in &block.transactions {
                        if let Some(v) = txs_table.get(tx.hash.as_slice()).map_err(map_err)? {
                            if let Ok(tx) = bincode::deserialize::<Transaction>(v.value()) {
                                result.push(tx);
                            }
                        }
                    }
                }
            }
        }
        Ok(result)
    }

    fn get_block_hashes_by_height_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<[u8; 32]>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
        let mut hashes = Vec::new();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (_, v) = item.map_err(map_err)?;
            if let Ok(block) = bincode::deserialize::<Block>(v.value()) {
                if block.index >= from && block.index <= to {
                    hashes.push(block.hash);
                }
            }
        }
        Ok(hashes)
    }

    fn get_account_transaction_count(&self, address: &PublicKey) -> Result<u64, StorageError> {
        let txs = self.get_transactions_by_address(address, usize::MAX)?;
        Ok(txs.len() as u64)
    }

    fn get_contract_transactions(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let mut txs = Vec::new();
        let cid_bytes = contract_id.to_bytes();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (_, v) = item.map_err(map_err)?;
            if let Ok(tx) = bincode::deserialize::<Transaction>(v.value()) {
                if let crate::types::Address::Contract(c) = &tx.recipient {
                    if c.to_bytes() == cid_bytes && txs.len() < limit {
                        txs.push(tx);
                    }
                }
            }
        }
        Ok(txs)
    }

    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError> {
        let key = address.to_bytes();
        let val = bincode::serialize(account)?;
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            table.insert(key.as_slice(), val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError> {
        let key = address.to_bytes();
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
        match table.get(key.as_slice()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError> {
        let key = address.to_bytes();
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            table.remove(key.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
        let mut accounts = Vec::new();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (k, v) = item.map_err(map_err)?;
            let key_bytes = k.value();
            // Skip chain state key
            if key_bytes == CHAIN_STATE_KEY {
                continue;
            }
            if key_bytes.len() == 32 {
                if let Ok(pk) = PublicKey::from_bytes(key_bytes.try_into().unwrap_or(&[0; 32])) {
                    if let Ok(account) = bincode::deserialize::<Account>(v.value()) {
                        accounts.push((pk, account));
                    }
                }
            }
        }
        Ok(accounts)
    }

    fn put_chain_state(&self, state: &ChainState) -> Result<(), StorageError> {
        let val = bincode::serialize(state)?;
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            table.insert(CHAIN_STATE_KEY, val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
        match table.get(CHAIN_STATE_KEY).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn put_contract_code(
        &self,
        contract_id: &ContractId,
        wasm_bytes: &[u8],
    ) -> Result<(), StorageError> {
        let key = format!("code:{}", hex::encode(contract_id.to_bytes()));
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            table.insert(key.as_bytes(), wasm_bytes).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        let key = format!("code:{}", hex::encode(contract_id.to_bytes()));
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
        match table.get(key.as_bytes()).map_err(map_err)? {
            Some(v) => Ok(Some(v.value().to_vec())),
            None => Ok(None),
        }
    }

    fn contract_storage_read(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let db_key = format!("cstore:{}:{}", hex::encode(contract_id.to_bytes()), hex::encode(key));
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
        match table.get(db_key.as_bytes()).map_err(map_err)? {
            Some(v) => Ok(Some(v.value().to_vec())),
            None => Ok(None),
        }
    }

    fn contract_storage_write(
        &self,
        contract_id: &ContractId,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StorageError> {
        let db_key = format!("cstore:{}:{}", hex::encode(contract_id.to_bytes()), hex::encode(key));
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            table.insert(db_key.as_bytes(), value).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn contract_storage_remove(
        &self,
        contract_id: &ContractId,
        key: &[u8],
    ) -> Result<(), StorageError> {
        let db_key = format!("cstore:{}:{}", hex::encode(contract_id.to_bytes()), hex::encode(key));
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            table.remove(db_key.as_bytes()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn contract_emit_event(
        &self,
        contract_id: &ContractId,
        topic: &[u8],
        data: &[u8],
    ) -> Result<(), StorageError> {
        let key = format!("event:{}:{}", hex::encode(contract_id.to_bytes()), hex::encode(topic));
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(EVENTS_TABLE).map_err(map_err)?;
            table.insert(key.as_bytes(), data).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_contract_events(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<ContractEvents, StorageError> {
        let prefix = format!("event:{}:", hex::encode(contract_id.to_bytes()));
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(EVENTS_TABLE).map_err(map_err)?;
        let mut events = Vec::new();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (k, v) = item.map_err(map_err)?;
            let key = String::from_utf8_lossy(k.value());
            if key.starts_with(&prefix) && events.len() < limit {
                let topic =
                    key.strip_prefix(&prefix).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
                events.push((topic, v.value().to_vec()));
            }
        }
        Ok(events)
    }

    fn get_all_contract_storage_keys(
        &self,
        contract_id: &ContractId,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        let prefix = format!("cstore:{}:", hex::encode(contract_id.to_bytes()));
        let txn = self.db.begin_read().map_err(map_err)?;
        let table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
        let mut keys = Vec::new();
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (k, _) = item.map_err(map_err)?;
            let key = String::from_utf8_lossy(k.value());
            if key.starts_with(&prefix) {
                let contract_key = key[prefix.len()..].to_string();
                keys.push(hex::decode(&contract_key).unwrap_or_default());
            }
        }
        Ok(keys)
    }

    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError> {
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut blocks = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
            let mut txs_table = txn.open_table(TXS_TABLE).map_err(map_err)?;
            let mut accounts = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            let mut contracts = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            let mut pending = txn.open_table(PENDING_TABLE).map_err(map_err)?;

            for op in &batch.ops {
                match op {
                    StorageOperation::PutBlock(k, v) => {
                        blocks.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutAccount(k, v) => {
                        accounts.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutChainState(k, v) => {
                        accounts.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutTransaction(k, v) => {
                        txs_table.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutTxIndex(k, v) => {
                        txs_table.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutContractCode(k, v) => {
                        contracts.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutContractStorage(k, v) => {
                        contracts.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::PutMempool(k, v) => {
                        pending.insert(k.as_slice(), v.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::DeleteMempool(k) => {
                        pending.remove(k.as_slice()).map_err(map_err)?;
                    }
                    StorageOperation::DeleteTransaction(k) => {
                        txs_table.remove(k.as_slice()).map_err(map_err)?;
                    }
                }
            }
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn compact(&self) -> Result<(), StorageError> {
        Ok(())
    }

    fn get_storage_stats(&self) -> Result<StorageStats, StorageError> {
        let txn = self.db.begin_read().map_err(map_err)?;
        let blocks = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
        let txs = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let accounts = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;

        let block_count = blocks.iter().map_err(map_err)?.count() as u64;
        let tx_count = txs.iter().map_err(map_err)?.count() as u64;
        let account_count = accounts.iter().map_err(map_err)?.count() as u64;
        drop(accounts);
        drop(txs);
        drop(blocks);
        drop(txn);

        Ok(StorageStats {
            total_blocks: block_count,
            total_transactions: tx_count,
            total_accounts: account_count,
            total_contracts: 0,
            mempool_size: 0,
            storage_size_bytes: 0,
            index_count: 0,
        })
    }

    fn clear_pending_transactions(&self) -> Result<(), StorageError> {
        let txn = self.db.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(PENDING_TABLE).map_err(map_err)?;
            let keys: Vec<Vec<u8>> = {
                let iter = table.iter().map_err(map_err)?;
                iter.map(|i| {
                    let (k, _) = i.map_err(map_err)?;
                    Ok(k.value().to_vec())
                })
                .collect::<Result<_, StorageError>>()?
            };
            for key in keys {
                table.remove(key.as_slice()).map_err(map_err)?;
            }
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn clone_storage(&self) -> Box<dyn Storage> {
        Box::new(Self { db: Arc::clone(&self.db) })
    }

    fn backup_to(&self, path: &std::path::Path) -> Result<(), StorageError> {
        use std::io::Write;
        let txn = self.db.begin_read().map_err(map_err)?;
        let mut file = std::fs::File::create(path).map_err(map_err)?;
        let tables = [
            ("blocks", BLOCKS_TABLE),
            ("txs", TXS_TABLE),
            ("pending", PENDING_TABLE),
            ("accounts", ACCOUNTS_TABLE),
            ("contracts", CONTRACTS_TABLE),
            ("events", EVENTS_TABLE),
        ];
        for (name, def) in &tables {
            let table = txn.open_table(*def).map_err(map_err)?;
            let iter = table.iter().map_err(map_err)?;
            for item in iter {
                let (k, v) = item.map_err(map_err)?;
                writeln!(file, "{}:{}:{}", name, hex::encode(k.value()), hex::encode(v.value()))
                    .map_err(map_err)?;
            }
        }
        Ok(())
    }

    fn restore_from(&self, path: &std::path::Path) -> Result<(), StorageError> {
        let content = std::fs::read_to_string(path).map_err(map_err)?;
        let txn = self.db.begin_write().map_err(map_err)?;
        for line in content.lines() {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() != 3 {
                continue;
            }
            let key = hex::decode(parts[1]).map_err(map_err)?;
            let val = hex::decode(parts[2]).map_err(map_err)?;
            let table_def: TableDefinition<&[u8], &[u8]> = match parts[0] {
                "blocks" => BLOCKS_TABLE,
                "txs" => TXS_TABLE,
                "pending" => PENDING_TABLE,
                "accounts" => ACCOUNTS_TABLE,
                "contracts" => CONTRACTS_TABLE,
                "events" => EVENTS_TABLE,
                _ => continue,
            };
            let mut table = txn.open_table(table_def).map_err(map_err)?;
            table.insert(key.as_slice(), val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }
}

impl Clone for RedbStorage {
    fn clone(&self) -> Self {
        Self { db: Arc::clone(&self.db) }
    }
}
