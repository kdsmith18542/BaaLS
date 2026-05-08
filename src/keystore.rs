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
const PBKDF2_ITER: u32 = 600_000;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

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

    pub fn load_key(&self, pk: &PublicKey, password: &str) -> Result<SigningKey, KeystoreError> {
        let path = self.key_path(pk);
        // Reject symlinks for security
        if path.is_symlink() {
            return Err(KeystoreError::Crypto("Key path is a symlink — rejected for security".to_string()));
        }
        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        if data.len() < SALT_LEN + NONCE_LEN {
            return Err(KeystoreError::Crypto("Corrupt key file".to_string()));
        }
        let salt = &data[..SALT_LEN];
        let nonce = &data[SALT_LEN..SALT_LEN + NONCE_LEN];
        let ciphertext = &data[SALT_LEN + NONCE_LEN..];
        let mut key = [0u8; KEY_LEN];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ITER, &mut key);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| KeystoreError::Crypto("Invalid key".to_string()))?;
        let mut plaintext = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| KeystoreError::InvalidPassword)?;
        key.zeroize();
        if plaintext.len() != 32 {
            plaintext.zeroize();
            return Err(KeystoreError::Crypto("Invalid key length".to_string()));
        }
        let mut sk_bytes = [0u8; 32];
        sk_bytes.copy_from_slice(&plaintext);
        plaintext.zeroize();
        let signing_key = SigningKey::from_bytes(&sk_bytes);
        sk_bytes.zeroize();
        Ok(signing_key)
    }

    fn save_key(&self, sk: &SigningKey, password: &str) -> Result<(), KeystoreError> {
        let pk = PublicKey::from_bytes(&sk.verifying_key().to_bytes())
            .map_err(|_| KeystoreError::Crypto("Invalid public key".to_string()))?;
        let path = self.key_path(&pk);

        // Reject symlinks to prevent symlink attacks
        if path.exists() {
            if path.is_symlink() {
                return Err(KeystoreError::Crypto("Key path is a symlink — rejected for security".to_string()));
            }
            return Err(KeystoreError::AlreadyExists(hex::encode(pk.to_bytes())));
        }

        let mut rng = rand::rng();
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut nonce);
        let mut key = [0u8; KEY_LEN];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITER, &mut key);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| KeystoreError::Crypto("Invalid key".to_string()))?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), sk.to_bytes().as_ref())
            .map_err(|_| KeystoreError::Crypto("Encryption failed".to_string()))?;
        key.zeroize();

        // Atomic write: write to temp file, then rename
        let tmp_path = path.with_extension("tmp");
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(&salt)?;
            file.write_all(&nonce)?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    fn key_path(&self, pk: &PublicKey) -> PathBuf {
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
}
