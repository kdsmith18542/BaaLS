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
const HEIGHT_TO_BLOCK_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("height_to_block");
const ADDR_TO_TX_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("addr_to_tx");
const CONTRACT_TO_TX_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("contract_to_tx");
const TX_COUNT_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("tx_count");

fn map_err(e: impl std::fmt::Display) -> StorageError {
    StorageError::IndexError(e.to_string())
}

pub struct RedbStorage {
    db: Arc<std::sync::RwLock<Arc<Database>>>,
    db_path: std::path::PathBuf,
}

impl RedbStorage {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let (db_path, db) = if path.is_dir() {
            std::fs::create_dir_all(path).map_err(map_err)?;
            let db_path = path.join("data.redb");
            let db = Database::create(&db_path).map_err(map_err)?;
            Self::init_tables(&db)?;
            (db_path, db)
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(map_err)?;
            }
            let db = Database::create(path).map_err(map_err)?;
            Self::init_tables(&db)?;
            (path.to_path_buf(), db)
        };
        Ok(Self { db: Arc::new(std::sync::RwLock::new(Arc::new(db))), db_path })
    }

    fn db_guard(&self) -> Result<std::sync::RwLockReadGuard<'_, Arc<Database>>, StorageError> {
        self.db.read().map_err(|e| StorageError::IndexError(e.to_string()))
    }

    #[allow(clippy::type_complexity)]
    fn read_all_tables(db: &Database) -> Result<Vec<(String, Vec<u8>, Vec<u8>)>, StorageError> {
        let txn = db.begin_read().map_err(map_err)?;
        #[allow(clippy::type_complexity)]
        let tables: [(&str, TableDefinition<&[u8], &[u8]>); 10] = [
            ("blocks", BLOCKS_TABLE),
            ("txs", TXS_TABLE),
            ("pending", PENDING_TABLE),
            ("accounts", ACCOUNTS_TABLE),
            ("contracts", CONTRACTS_TABLE),
            ("events", EVENTS_TABLE),
            ("height_to_block", HEIGHT_TO_BLOCK_TABLE),
            ("addr_to_tx", ADDR_TO_TX_TABLE),
            ("contract_to_tx", CONTRACT_TO_TX_TABLE),
            ("tx_count", TX_COUNT_TABLE),
        ];
        let mut data = Vec::new();
        for (name, def) in &tables {
            let table = txn.open_table(*def).map_err(map_err)?;
            let iter = table.iter().map_err(map_err)?;
            for item in iter {
                let (k, v) = item.map_err(map_err)?;
                data.push((name.to_string(), k.value().to_vec(), v.value().to_vec()));
            }
        }
        Ok(data)
    }

    fn write_all_tables(
        db: &Database,
        data: Vec<(String, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StorageError> {
        let txn = db.begin_write().map_err(map_err)?;
        {
            let mut blocks = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
            let mut txs = txn.open_table(TXS_TABLE).map_err(map_err)?;
            let mut pending = txn.open_table(PENDING_TABLE).map_err(map_err)?;
            let mut accounts = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            let mut contracts = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            let mut events = txn.open_table(EVENTS_TABLE).map_err(map_err)?;
            let mut height_to_block = txn.open_table(HEIGHT_TO_BLOCK_TABLE).map_err(map_err)?;
            let mut addr_to_tx = txn.open_table(ADDR_TO_TX_TABLE).map_err(map_err)?;
            let mut contract_to_tx = txn.open_table(CONTRACT_TO_TX_TABLE).map_err(map_err)?;
            let mut tx_count = txn.open_table(TX_COUNT_TABLE).map_err(map_err)?;

            for (table_name, key, value) in data {
                match table_name.as_str() {
                    "blocks" => {
                        blocks.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "txs" => {
                        txs.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "pending" => {
                        pending.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "accounts" => {
                        accounts.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "contracts" => {
                        contracts.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "events" => {
                        events.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "height_to_block" => {
                        height_to_block
                            .insert(key.as_slice(), value.as_slice())
                            .map_err(map_err)?;
                    }
                    "addr_to_tx" => {
                        addr_to_tx.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "contract_to_tx" => {
                        contract_to_tx.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    "tx_count" => {
                        tx_count.insert(key.as_slice(), value.as_slice()).map_err(map_err)?;
                    }
                    _ => {}
                }
            }
        }
        txn.commit().map_err(map_err)?;
        Ok(())
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
            txn.open_table(HEIGHT_TO_BLOCK_TABLE).map_err(map_err)?;
            txn.open_table(ADDR_TO_TX_TABLE).map_err(map_err)?;
            txn.open_table(CONTRACT_TO_TX_TABLE).map_err(map_err)?;
            txn.open_table(TX_COUNT_TABLE).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }
}

impl Storage for RedbStorage {
    fn put_block(&self, block: &Block) -> Result<(), StorageError> {
        let val = bincode::serialize(block)?;
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
            table.insert(block.hash.as_slice(), val.as_slice()).map_err(map_err)?;
            let mut height_table = txn.open_table(HEIGHT_TO_BLOCK_TABLE).map_err(map_err)?;
            let height_key = format!("height:{}", block.index);
            height_table.insert(height_key.as_bytes(), block.hash.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let height_table = txn.open_table(HEIGHT_TO_BLOCK_TABLE).map_err(map_err)?;
        let height_key = format!("height:{}", height);
        match height_table.get(height_key.as_bytes()).map_err(map_err)? {
            Some(hash_entry) => {
                let mut block_hash = [0u8; 32];
                let hash_bytes = hash_entry.value();
                if hash_bytes.len() == 32 {
                    block_hash.copy_from_slice(hash_bytes);
                }
                let blocks = txn.open_table(BLOCKS_TABLE).map_err(map_err)?;
                match blocks.get(block_hash.as_slice()).map_err(map_err)? {
                    Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    fn put_transaction(&self, tx: &Transaction) -> Result<(), StorageError> {
        let val = bincode::serialize(tx)?;
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(TXS_TABLE).map_err(map_err)?;
            table.insert(tx.hash.as_slice(), val.as_slice()).map_err(map_err)?;

            // Index by sender address
            let mut addr_table = txn.open_table(ADDR_TO_TX_TABLE).map_err(map_err)?;
            let addr_key = format!(
                "{}:{:0>20}:{}",
                hex::encode(tx.sender.to_bytes()),
                tx.timestamp,
                hex::encode(tx.hash)
            );
            addr_table.insert(addr_key.as_bytes(), tx.hash.as_slice()).map_err(map_err)?;

            // Index by contract recipient
            if let crate::types::Address::Contract(contract_id) = &tx.recipient {
                let mut contract_table = txn.open_table(CONTRACT_TO_TX_TABLE).map_err(map_err)?;
                let contract_key = format!(
                    "{}:{:0>20}:{}",
                    hex::encode(contract_id.to_bytes()),
                    tx.timestamp,
                    hex::encode(tx.hash)
                );
                contract_table
                    .insert(contract_key.as_bytes(), tx.hash.as_slice())
                    .map_err(map_err)?;
            }

            // Increment transaction count
            let mut count_table = txn.open_table(TX_COUNT_TABLE).map_err(map_err)?;
            let count_key = hex::encode(tx.sender.to_bytes());
            let current_count = count_table
                .get(count_key.as_bytes())
                .map_err(map_err)?
                .map(|v| String::from_utf8_lossy(v.value()).parse::<u64>().unwrap_or(0))
                .unwrap_or(0);
            count_table
                .insert(count_key.as_bytes(), (current_count + 1).to_string().as_bytes())
                .map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_transaction(&self, tx_hash: &[u8; 32]) -> Result<Option<Transaction>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        match table.get(tx_hash.as_slice()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn get_pending_transactions(&self) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
    ) -> Result<Option<(Block, Transaction)>, StorageError> {
        // Get the transaction
        let tx = match self.get_transaction(tx_hash)? {
            Some(t) => t,
            None => return Ok(None),
        };
        // Find block hash from tx index: scan prefix "idx:{tx_hash}:"
        let prefix = format!("idx:{}:", hex::encode(tx_hash));
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let mut block_hash = None;
        let iter = table.iter().map_err(map_err)?;
        for item in iter {
            let (k, v) = item.map_err(map_err)?;
            let key_str = String::from_utf8_lossy(k.value());
            if key_str.starts_with(&prefix) && v.value().len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(v.value());
                block_hash = Some(arr);
                break;
            }
        }
        match block_hash {
            Some(bh) => match self.get_block(&bh)? {
                Some(block) => Ok(Some((block, tx))),
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    fn get_transactions_by_block(
        &self,
        _block_hash: &[u8; 32],
    ) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let addr_table = txn.open_table(ADDR_TO_TX_TABLE).map_err(map_err)?;
        let txs_table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let prefix = format!("{}:", hex::encode(address.to_bytes()));
        let mut txs = Vec::new();
        let iter = addr_table.iter().map_err(map_err)?;
        for item in iter {
            let (k, v) = item.map_err(map_err)?;
            let key_str = String::from_utf8_lossy(k.value());
            if key_str.starts_with(&prefix) {
                let tx_hash = v.value();
                if let Ok(tx) = bincode::deserialize::<Transaction>(
                    txs_table.get(tx_hash).map_err(map_err)?.unwrap().value(),
                ) {
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let table = txn.open_table(TX_COUNT_TABLE).map_err(map_err)?;
        let key = hex::encode(address.to_bytes());
        match table.get(key.as_bytes()).map_err(map_err)? {
            Some(v) => {
                let count = String::from_utf8_lossy(v.value()).parse::<u64>().unwrap_or(0);
                Ok(count)
            }
            None => Ok(0),
        }
    }

    fn get_contract_transactions(
        &self,
        contract_id: &ContractId,
        limit: usize,
    ) -> Result<Vec<Transaction>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let contract_table = txn.open_table(CONTRACT_TO_TX_TABLE).map_err(map_err)?;
        let txs_table = txn.open_table(TXS_TABLE).map_err(map_err)?;
        let prefix = format!("{}:", hex::encode(contract_id.to_bytes()));
        let mut txs = Vec::new();
        let iter = contract_table.iter().map_err(map_err)?;
        for item in iter {
            let (k, v) = item.map_err(map_err)?;
            let key_str = String::from_utf8_lossy(k.value());
            if key_str.starts_with(&prefix) {
                let tx_hash = v.value();
                if let Ok(tx) = bincode::deserialize::<Transaction>(
                    txs_table.get(tx_hash).map_err(map_err)?.unwrap().value(),
                ) {
                    txs.push(tx);
                    if txs.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(txs)
    }

    fn put_account(&self, address: &PublicKey, account: &Account) -> Result<(), StorageError> {
        let key = address.to_bytes();
        let val = bincode::serialize(account)?;
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            table.insert(key.as_slice(), val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_account(&self, address: &PublicKey) -> Result<Option<Account>, StorageError> {
        let key = address.to_bytes();
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
        match table.get(key.as_slice()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn delete_account(&self, address: &PublicKey) -> Result<(), StorageError> {
        let key = address.to_bytes();
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            table.remove(key.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_all_accounts(&self) -> Result<Vec<(PublicKey, Account)>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(ACCOUNTS_TABLE).map_err(map_err)?;
            table.insert(CHAIN_STATE_KEY, val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_chain_state(&self) -> Result<Option<ChainState>, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            table.insert(key.as_bytes(), wasm_bytes).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_contract_code(&self, contract_id: &ContractId) -> Result<Option<Vec<u8>>, StorageError> {
        let key = format!("code:{}", hex::encode(contract_id.to_bytes()));
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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

    fn put_contract_deployer(
        &self,
        contract_id: &ContractId,
        deployer: &PublicKey,
    ) -> Result<(), StorageError> {
        let key = format!("deployer:{}", hex::encode(contract_id.to_bytes()));
        let val = bincode::serialize(deployer)?;
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
        {
            let mut table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
            table.insert(key.as_bytes(), val.as_slice()).map_err(map_err)?;
        }
        txn.commit().map_err(map_err)?;
        Ok(())
    }

    fn get_contract_deployer(
        &self,
        contract_id: &ContractId,
    ) -> Result<Option<PublicKey>, StorageError> {
        let key = format!("deployer:{}", hex::encode(contract_id.to_bytes()));
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let table = txn.open_table(CONTRACTS_TABLE).map_err(map_err)?;
        match table.get(key.as_bytes()).map_err(map_err)? {
            Some(v) => Ok(Some(bincode::deserialize(v.value())?)),
            None => Ok(None),
        }
    }

    fn apply_batch(&self, batch: StorageBatch) -> Result<(), StorageError> {
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        // Read all data from the current database
        let data = {
            let db = self.db_guard()?;
            Self::read_all_tables(&db)?
        };

        let backup_path = self.db_path.with_extension("redb.backup");

        // Rename current db file to backup
        std::fs::rename(&self.db_path, &backup_path).map_err(|e| {
            StorageError::IndexError(format!("Failed to rename db to backup: {}", e))
        })?;

        let result = (|| -> Result<(), StorageError> {
            let new_db = Database::create(&self.db_path).map_err(map_err)?;
            Self::init_tables(&new_db)?;
            Self::write_all_tables(&new_db, data)?;

            let mut db_guard =
                self.db.write().map_err(|e| StorageError::IndexError(e.to_string()))?;
            *db_guard = Arc::new(new_db);

            Ok(())
        })();

        if result.is_err() {
            if let Err(e) = std::fs::rename(&backup_path, &self.db_path) {
                log::error!("Failed to restore backup after compaction failure: {}", e);
            }
        } else {
            if let Err(e) = std::fs::remove_file(&backup_path) {
                log::warn!("Failed to remove backup file after compaction: {}", e);
            }
        }

        result
    }

    fn get_storage_stats(&self) -> Result<StorageStats, StorageError> {
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
        Box::new(Self { db: Arc::clone(&self.db), db_path: self.db_path.clone() })
    }

    fn backup_to(&self, path: &std::path::Path) -> Result<(), StorageError> {
        use std::io::Write;
        let txn = self.db_guard()?.begin_read().map_err(map_err)?;
        let mut file = std::fs::File::create(path).map_err(map_err)?;
        let tables = [
            ("blocks", BLOCKS_TABLE),
            ("txs", TXS_TABLE),
            ("pending", PENDING_TABLE),
            ("accounts", ACCOUNTS_TABLE),
            ("contracts", CONTRACTS_TABLE),
            ("events", EVENTS_TABLE),
            ("height_to_block", HEIGHT_TO_BLOCK_TABLE),
            ("addr_to_tx", ADDR_TO_TX_TABLE),
            ("contract_to_tx", CONTRACT_TO_TX_TABLE),
            ("tx_count", TX_COUNT_TABLE),
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
        let txn = self.db_guard()?.begin_write().map_err(map_err)?;
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
                "height_to_block" => HEIGHT_TO_BLOCK_TABLE,
                "addr_to_tx" => ADDR_TO_TX_TABLE,
                "contract_to_tx" => CONTRACT_TO_TX_TABLE,
                "tx_count" => TX_COUNT_TABLE,
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
        Self { db: Arc::clone(&self.db), db_path: self.db_path.clone() }
    }
}
