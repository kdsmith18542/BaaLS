use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroize;

use crate::types::PublicKey;
use ed25519_dalek::SigningKey;

const KEYSTORE_DIR: &str = ".baals/keys";
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

// V1 (PBKDF2) constants
const PBKDF2_ITER: u32 = 600_000;

// V2 (Argon2id) constants
const ARGON2_MEM_COST: u32 = 19 * 1024; // 19 MiB (OWASP recommended minimum)
const ARGON2_TIME_COST: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;
const ARGON2_SALT_LEN: usize = 16;

// Format version markers
const FORMAT_MAGIC: u8 = 0xBA;
const FORMAT_V1: u8 = 0x01;
const FORMAT_V2: u8 = 0x02;

#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Crypto error: {0}")]
    Crypto(String),
    #[error("Key not found: {0}")]
    NotFound(String),
    #[error("Invalid password")]
    InvalidPassword,
    #[error("Key already exists: {0}")]
    AlreadyExists(String),
}

enum KeystoreFormat {
    V1, // PBKDF2-SHA256
    V2, // Argon2id
}

pub struct Keystore {
    dir: PathBuf,
}

impl Keystore {
    pub fn new(base_dir: Option<PathBuf>) -> Result<Self, KeystoreError> {
        let home = dirs::home_dir().ok_or_else(|| {
            KeystoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Home dir not found",
            ))
        })?;
        let dir = base_dir.unwrap_or_else(|| home.join(KEYSTORE_DIR));
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn list_keys(&self) -> Result<Vec<PublicKey>, KeystoreError> {
        let mut keys = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(pk) = Self::pubkey_from_filename(&path) {
                    keys.push(pk);
                }
            }
        }
        Ok(keys)
    }

    pub fn create_key(&self, password: &str) -> Result<PublicKey, KeystoreError> {
        let mut rng = rand::rng();
        let mut sk_bytes = [0u8; 32];
        rng.fill_bytes(&mut sk_bytes);
        let sk = SigningKey::from_bytes(&sk_bytes);
        let pk = PublicKey::from_bytes(&sk.verifying_key().to_bytes())
            .map_err(|_| KeystoreError::Crypto("Invalid public key".to_string()))?;
        self.save_key(&sk, password)?;
        sk_bytes.zeroize();
        Ok(pk)
    }

    pub fn import_key(
        &self,
        sk_bytes: &[u8; 32],
        password: &str,
    ) -> Result<PublicKey, KeystoreError> {
        let sk = SigningKey::from_bytes(sk_bytes);
        let pk = PublicKey::from_bytes(&sk.verifying_key().to_bytes())
            .map_err(|_| KeystoreError::Crypto("Invalid public key".to_string()))?;
        self.save_key(&sk, password)?;
        Ok(pk)
    }

    pub fn export_key(&self, pk: &PublicKey, password: &str) -> Result<[u8; 32], KeystoreError> {
        let sk = self.load_key(pk, password)?;
        Ok(sk.to_bytes())
    }

    pub fn delete_key(&self, pk: &PublicKey) -> Result<(), KeystoreError> {
        let path = self.key_path(pk);
        if path.exists() {
            fs::remove_file(path)?;
            Ok(())
        } else {
            Err(KeystoreError::NotFound(hex::encode(pk.to_bytes())))
        }
    }

    pub fn key_format_version(&self, pk: &PublicKey) -> Result<u8, KeystoreError> {
        let path = self.key_path(pk);
        if !path.exists() {
            return Err(KeystoreError::NotFound(hex::encode(pk.to_bytes())));
        }
        let mut data = Vec::new();
        File::open(&path)?.read_to_end(&mut data)?;
        Ok(Self::detect_format(&data).map_or(1, |f| match f {
            KeystoreFormat::V1 => 1,
            KeystoreFormat::V2 => 2,
        }))
    }

    pub fn load_key(&self, pk: &PublicKey, password: &str) -> Result<SigningKey, KeystoreError> {
        let path = self.key_path(pk);
        if path.is_symlink() {
            return Err(KeystoreError::Crypto(
                "Key path is a symlink — rejected for security".to_string(),
            ));
        }
        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        match Self::detect_format(&data) {
            Some(KeystoreFormat::V2) => self.load_key_v2(&data, password),
            _ => self.load_key_v1(&data, password),
        }
    }

    fn load_key_v1(&self, data: &[u8], password: &str) -> Result<SigningKey, KeystoreError> {
        let offset =
            if data.len() > 2 && data[0] == FORMAT_MAGIC && data[1] == FORMAT_V1 { 2 } else { 0 };
        let remaining = &data[offset..];
        if remaining.len() < SALT_LEN + NONCE_LEN {
            return Err(KeystoreError::Crypto("Corrupt key file".to_string()));
        }
        let salt = &remaining[..SALT_LEN];
        let nonce = &remaining[SALT_LEN..SALT_LEN + NONCE_LEN];
        let ciphertext = &remaining[SALT_LEN + NONCE_LEN..];
        let mut key = [0u8; KEY_LEN];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ITER, &mut key);
        let plaintext = self.decrypt(key, nonce, ciphertext)?;
        Self::bytes_to_signing_key(plaintext)
    }

    fn load_key_v2(&self, data: &[u8], password: &str) -> Result<SigningKey, KeystoreError> {
        // Format: magic(1) + version(1) + mem_cost(4) + time_cost(1) + parallelism(1) + salt(16) + nonce(12) + ciphertext
        if data.len() < 2 + 4 + 1 + 1 + ARGON2_SALT_LEN + NONCE_LEN {
            return Err(KeystoreError::Crypto("Corrupt v2 key file".to_string()));
        }
        let mut mem_cost_bytes = [0u8; 4];
        mem_cost_bytes.copy_from_slice(&data[2..6]);
        let mem_cost = u32::from_le_bytes(mem_cost_bytes);
        let time_cost = data[6] as u32;
        let parallelism = data[7] as u32;
        let salt = &data[8..8 + ARGON2_SALT_LEN];
        let nonce = &data[8 + ARGON2_SALT_LEN..8 + ARGON2_SALT_LEN + NONCE_LEN];
        let ciphertext = &data[8 + ARGON2_SALT_LEN + NONCE_LEN..];

        let mut key = [0u8; KEY_LEN];
        argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(mem_cost, time_cost, parallelism, Some(KEY_LEN))
                .map_err(|e| KeystoreError::Crypto(format!("Argon2 params: {}", e)))?,
        )
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| KeystoreError::Crypto(format!("Argon2 failed: {}", e)))?;

        let plaintext = self.decrypt(key, nonce, ciphertext)?;
        Self::bytes_to_signing_key(plaintext)
    }

    fn decrypt(
        &self,
        mut key: [u8; KEY_LEN],
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, KeystoreError> {
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| KeystoreError::Crypto("Invalid key".to_string()))?;
        let result = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| KeystoreError::InvalidPassword);
        key.zeroize();
        result
    }

    fn bytes_to_signing_key(plaintext: Vec<u8>) -> Result<SigningKey, KeystoreError> {
        let mut pt = plaintext;
        if pt.len() != 32 {
            pt.zeroize();
            return Err(KeystoreError::Crypto("Invalid key length".to_string()));
        }
        let mut sk_bytes = [0u8; 32];
        sk_bytes.copy_from_slice(&pt);
        pt.zeroize();
        let signing_key = SigningKey::from_bytes(&sk_bytes);
        sk_bytes.zeroize();
        Ok(signing_key)
    }

    fn save_key(&self, sk: &SigningKey, password: &str) -> Result<(), KeystoreError> {
        let pk = PublicKey::from_bytes(&sk.verifying_key().to_bytes())
            .map_err(|_| KeystoreError::Crypto("Invalid public key".to_string()))?;
        let path = self.key_path(&pk);

        if path.exists() {
            if path.is_symlink() {
                return Err(KeystoreError::Crypto(
                    "Key path is a symlink — rejected for security".to_string(),
                ));
            }
            return Err(KeystoreError::AlreadyExists(hex::encode(pk.to_bytes())));
        }

        let mut rng = rand::rng();
        let mut nonce = [0u8; NONCE_LEN];
        rng.fill_bytes(&mut nonce);

        let mut key = [0u8; KEY_LEN];
        let mut salt = [0u8; ARGON2_SALT_LEN];
        rng.fill_bytes(&mut salt);

        argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(
                ARGON2_MEM_COST,
                ARGON2_TIME_COST,
                ARGON2_PARALLELISM,
                Some(KEY_LEN),
            )
            .map_err(|e| KeystoreError::Crypto(format!("Argon2 params: {}", e)))?,
        )
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| KeystoreError::Crypto(format!("Argon2 failed: {}", e)))?;

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| KeystoreError::Crypto("Invalid key".to_string()))?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), sk.to_bytes().as_ref())
            .map_err(|_| KeystoreError::Crypto("Encryption failed".to_string()))?;
        key.zeroize();

        // V2 format: magic + version + mem_cost + time_cost + parallelism + salt + nonce + ciphertext
        let mut file_data = Vec::new();
        file_data.push(FORMAT_MAGIC);
        file_data.push(FORMAT_V2);
        file_data.extend_from_slice(&ARGON2_MEM_COST.to_le_bytes());
        file_data.push(ARGON2_TIME_COST as u8);
        file_data.push(ARGON2_PARALLELISM as u8);
        file_data.extend_from_slice(&salt);
        file_data.extend_from_slice(&nonce);
        file_data.extend_from_slice(&ciphertext);

        // Atomic write: write to temp file, then rename
        let tmp_path = path.with_extension("tmp");
        {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp_path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(&file_data)?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    fn detect_format(data: &[u8]) -> Option<KeystoreFormat> {
        if data.len() >= 2 && data[0] == FORMAT_MAGIC {
            match data[1] {
                FORMAT_V1 => Some(KeystoreFormat::V1),
                FORMAT_V2 => Some(KeystoreFormat::V2),
                _ => None,
            }
        } else {
            // No magic prefix = legacy V1 format
            Some(KeystoreFormat::V1)
        }
    }

    pub fn key_path(&self, pk: &PublicKey) -> PathBuf {
        self.dir.join(hex::encode(pk.to_bytes()))
    }

    fn pubkey_from_filename(path: &Path) -> Option<PublicKey> {
        if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
            if let Ok(bytes) = hex::decode(fname) {
                if bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    return PublicKey::from_bytes(&arr).ok();
                }
            }
        }
        None
    }

    pub fn consensus_key_path(data_dir: &std::path::Path) -> std::path::PathBuf {
        data_dir.join("consensus.key.enc")
    }

    pub fn generate_consensus_key(
        data_dir: &std::path::Path,
        password: &str,
    ) -> Result<SigningKey, KeystoreError> {
        let mut rng = rand::rng();
        let mut sk_bytes = [0u8; 32];
        rng.fill_bytes(&mut sk_bytes);
        let sk = SigningKey::from_bytes(&sk_bytes);
        let _pk = PublicKey::from_bytes(&sk.verifying_key().to_bytes())
            .map_err(|_| KeystoreError::Crypto("Invalid public key".to_string()))?;
        let path = Self::consensus_key_path(data_dir);

        let mut nonce = [0u8; NONCE_LEN];
        rng.fill_bytes(&mut nonce);

        let mut key = [0u8; KEY_LEN];
        let mut salt = [0u8; ARGON2_SALT_LEN];
        rng.fill_bytes(&mut salt);

        argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(
                ARGON2_MEM_COST,
                ARGON2_TIME_COST,
                ARGON2_PARALLELISM,
                Some(KEY_LEN),
            )
            .map_err(|e| KeystoreError::Crypto(format!("Argon2 params: {}", e)))?,
        )
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| KeystoreError::Crypto(format!("Argon2 failed: {}", e)))?;

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| KeystoreError::Crypto("Invalid key".to_string()))?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), sk_bytes.as_ref())
            .map_err(|_| KeystoreError::Crypto("Encryption failed".to_string()))?;
        key.zeroize();
        sk_bytes.zeroize();

        let mut file_data = Vec::new();
        file_data.push(FORMAT_MAGIC);
        file_data.push(FORMAT_V2);
        file_data.extend_from_slice(&ARGON2_MEM_COST.to_le_bytes());
        file_data.push(ARGON2_TIME_COST as u8);
        file_data.push(ARGON2_PARALLELISM as u8);
        file_data.extend_from_slice(&salt);
        file_data.extend_from_slice(&nonce);
        file_data.extend_from_slice(&ciphertext);

        let tmp_path = path.with_extension("tmp");
        {
            let mut file =
                std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp_path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(&file_data)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp_path, &path)?;
        log::info!("Generated and encrypted consensus key at {:?}", path);

        let mut decrypted = [0u8; 32];
        decrypted.copy_from_slice(&sk.to_bytes());
        Ok(SigningKey::from_bytes(&decrypted))
    }

    pub fn load_consensus_key(
        data_dir: &std::path::Path,
        password: &str,
    ) -> Result<SigningKey, KeystoreError> {
        let path = Self::consensus_key_path(data_dir);
        if !path.exists() {
            return Err(KeystoreError::NotFound("Consensus key file not found".to_string()));
        }
        if path.is_symlink() {
            return Err(KeystoreError::Crypto(
                "Consensus key path is a symlink — rejected for security".to_string(),
            ));
        }
        let mut data = Vec::new();
        std::fs::File::open(&path)?.read_to_end(&mut data)?;

        if data.len() > 2 && data[0] == FORMAT_MAGIC && data[1] == FORMAT_V2 {
            let mut mem_cost_bytes = [0u8; 4];
            mem_cost_bytes.copy_from_slice(&data[2..6]);
            let mem_cost = u32::from_le_bytes(mem_cost_bytes);
            let time_cost = data[6] as u32;
            let parallelism = data[7] as u32;
            let salt = &data[8..8 + ARGON2_SALT_LEN];
            let nonce = &data[8 + ARGON2_SALT_LEN..8 + ARGON2_SALT_LEN + NONCE_LEN];
            let ciphertext = &data[8 + ARGON2_SALT_LEN + NONCE_LEN..];

            let mut key = [0u8; KEY_LEN];
            argon2::Argon2::new(
                argon2::Algorithm::Argon2id,
                argon2::Version::V0x13,
                argon2::Params::new(mem_cost, time_cost, parallelism, Some(KEY_LEN))
                    .map_err(|e| KeystoreError::Crypto(format!("Argon2 params: {}", e)))?,
            )
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| KeystoreError::Crypto(format!("Argon2 failed: {}", e)))?;

            let cipher = Aes256Gcm::new_from_slice(&key)
                .map_err(|_| KeystoreError::Crypto("Invalid key".to_string()))?;
            let plaintext = cipher
                .decrypt(Nonce::from_slice(nonce), ciphertext)
                .map_err(|_| KeystoreError::InvalidPassword)?;
            key.zeroize();

            if plaintext.len() != 32 {
                return Err(KeystoreError::Crypto("Invalid consensus key length".to_string()));
            }
            let mut sk_bytes = [0u8; 32];
            sk_bytes.copy_from_slice(&plaintext);
            Ok(SigningKey::from_bytes(&sk_bytes))
        } else if data.len() == 32 {
            log::warn!(
                "Loading consensus key from unencrypted file. \
                 Set BAALS_CONSENSUS_PASSWORD and remove consensus.key to use encrypted storage."
            );
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&data);
            Ok(SigningKey::from_bytes(&arr))
        } else {
            Err(KeystoreError::Crypto("Unrecognized consensus key format".to_string()))
        }
    }
}
