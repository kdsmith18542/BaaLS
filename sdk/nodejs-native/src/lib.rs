use std::path::{Path, PathBuf, Component};

use baals::config::StorageBackend;
use baals::sdk::BaaLSSdk;
use baals::types::{Account, PublicKey, Transaction};
use napi_derive::napi;

#[napi]
pub struct BaalsClient {
    inner: BaaLSSdk,
}

#[napi]
impl BaalsClient {
    #[napi(constructor)]
    pub fn new(data_dir: String, backend: Option<String>) -> napi::Result<Self> {
        let path = Path::new(&data_dir);
        if path.components().any(|c| c == Component::ParentDir) {
            return Err(napi::Error::from_reason(format!("Invalid input: {}", path.display())));
        }
        let backend = backend.as_deref().and_then(|s| {
            if s.eq_ignore_ascii_case("redb") { Some(StorageBackend::Redb) } else { None }
        });
        let sdk = BaaLSSdk::with_backend(PathBuf::from(data_dir), backend)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        Ok(Self { inner: sdk })
    }

    #[napi]
    pub fn start(&self) -> napi::Result<()> {
        self.inner.start().map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn stop(&self) -> napi::Result<()> {
        self.inner.stop().map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn chain_state_json(&self) -> napi::Result<String> {
        let state = self.inner.get_chain_state()
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        serde_json::to_string(&state)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn get_block_by_height(&self, height: f64) -> napi::Result<Option<String>> {
        let block = self.inner.get_block_by_height(height as u64)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        block.map(|b| serde_json::to_string(&b)
            .map_err(|e| napi::Error::from_reason(format!("{}", e))))
            .transpose()
    }

    #[napi]
    pub fn get_transaction(&self, hash_hex: String) -> napi::Result<Option<String>> {
        let hash_bytes = hex::decode(&hash_hex)
            .map_err(|e| napi::Error::from_reason(format!("Invalid hex: {}", e)))?;
        let mut hash = [0u8; 32];
        if hash_bytes.len() != 32 {
            return Err(napi::Error::from_reason("Hash must be 32 bytes"));
        }
        hash.copy_from_slice(&hash_bytes);
        let tx = self.inner.get_transaction(&hash)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        tx.map(|t| serde_json::to_string(&t)
            .map_err(|e| napi::Error::from_reason(format!("{}", e))))
            .transpose()
    }

    #[napi]
    pub fn get_account(&self, pubkey_hex: String) -> napi::Result<Option<String>> {
        let pk_bytes = hex::decode(&pubkey_hex)
            .map_err(|e| napi::Error::from_reason(format!("Invalid hex: {}", e)))?;
        let mut arr = [0u8; 32];
        if pk_bytes.len() != 32 {
            return Err(napi::Error::from_reason("Public key must be 32 bytes"));
        }
        arr.copy_from_slice(&pk_bytes);
        let pk = PublicKey::from_bytes(&arr)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        let acc = self.inner.get_account(&pk)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        acc.map(|a| serde_json::to_string(&a)
            .map_err(|e| napi::Error::from_reason(format!("{}", e))))
            .transpose()
    }

    #[napi]
    pub fn submit_tx(&self, tx_json: String) -> napi::Result<()> {
        let tx: Transaction = serde_json::from_str(&tx_json)
            .map_err(|e| napi::Error::from_reason(format!("Invalid transaction JSON: {}", e)))?;
        self.inner.submit_transaction(tx)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn create_account(&self, pubkey_hex: String, balance: f64) -> napi::Result<()> {
        let pk_bytes = hex::decode(&pubkey_hex)
            .map_err(|e| napi::Error::from_reason(format!("Invalid hex: {}", e)))?;
        let mut arr = [0u8; 32];
        if pk_bytes.len() != 32 {
            return Err(napi::Error::from_reason("Public key must be 32 bytes"));
        }
        arr.copy_from_slice(&pk_bytes);
        let pk = PublicKey::from_bytes(&arr)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))?;
        let account = Account::Wallet { balance: balance as u64, nonce: 0 };
        self.inner.create_account(&pk, account)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn add_peer(&self, address: String) -> napi::Result<()> {
        self.inner.add_peer(&address)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn remove_peer(&self, address: String) -> napi::Result<()> {
        self.inner.remove_peer(&address)
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }

    #[napi]
    pub fn known_peers(&self) -> Vec<String> {
        self.inner.known_peers()
    }

    #[napi]
    pub fn trigger_sync(&self) -> napi::Result<()> {
        self.inner.trigger_sync()
            .map_err(|e| napi::Error::from_reason(format!("{}", e)))
    }
}
