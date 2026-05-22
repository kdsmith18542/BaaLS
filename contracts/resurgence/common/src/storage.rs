use crate::host::{baals_storage_read, baals_storage_write, baals_storage_remove};
use serde::{Serialize, de::DeserializeOwned};

pub fn read<T: DeserializeOwned>(key: &[u8]) -> Option<T> {
    let mut buf = vec![0u8; 4096];
    let len = unsafe {
        baals_storage_read(
            key.as_ptr() as i32,
            key.len() as i32,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
        )
    };
    if len <= 0 {
        return None;
    }
    bincode::deserialize(&buf[..len as usize]).ok()
}

pub fn write<T: Serialize>(key: &[u8], value: &T) {
    if let Ok(serialized) = bincode::serialize(value) {
        unsafe {
            baals_storage_write(
                key.as_ptr() as i32,
                key.len() as i32,
                serialized.as_ptr() as i32,
                serialized.len() as i32,
            );
        }
    }
}

pub fn remove(key: &[u8]) {
    unsafe {
        baals_storage_remove(key.as_ptr() as i32, key.len() as i32);
    }
}

pub fn mapping_key(prefix: &[u8], key: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(prefix.len() + key.len());
    v.extend_from_slice(prefix);
    v.extend_from_slice(key);
    v
}

pub fn mapping_key_2(prefix: &[u8], key1: &[u8], key2: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(prefix.len() + key1.len() + key2.len());
    v.extend_from_slice(prefix);
    v.extend_from_slice(key1);
    v.extend_from_slice(key2);
    v
}

pub struct StorageValue<T> {
    key: &'static [u8],
    _marker: std::marker::PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned> StorageValue<T> {
    pub const fn new(key: &'static [u8]) -> Self {
        Self {
            key,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn get(&self) -> Option<T> {
        read(self.key)
    }

    pub fn get_or_default(&self) -> T where T: Default {
        self.get().unwrap_or_default()
    }

    pub fn set(&self, value: &T) {
        write(self.key, value);
    }

    pub fn remove(&self) {
        remove(self.key);
    }
}

pub struct StorageMap<K, V> {
    prefix: &'static [u8],
    _marker: std::marker::PhantomData<(K, V)>,
}

impl<K: Serialize, V: Serialize + DeserializeOwned> StorageMap<K, V> {
    pub const fn new(prefix: &'static [u8]) -> Self {
        Self {
            prefix,
            _marker: std::marker::PhantomData,
        }
    }

    fn key(&self, key: &K) -> Vec<u8> {
        let key_bytes = bincode::serialize(key).unwrap_or_default();
        mapping_key(self.prefix, &key_bytes)
    }

    pub fn get(&self, key: &K) -> Option<V> {
        read(&self.key(key))
    }

    pub fn get_or_default(&self, key: &K) -> V where V: Default {
        self.get(key).unwrap_or_default()
    }

    pub fn set(&self, key: &K, value: &V) {
        write(&self.key(key), value);
    }

    pub fn remove(&self, key: &K) {
        remove(&self.key(key));
    }
}

pub struct StorageDoubleMap<K1, K2, V> {
    prefix: &'static [u8],
    _marker: std::marker::PhantomData<(K1, K2, V)>,
}

impl<K1: Serialize, K2: Serialize, V: Serialize + DeserializeOwned> StorageDoubleMap<K1, K2, V> {
    pub const fn new(prefix: &'static [u8]) -> Self {
        Self {
            prefix,
            _marker: std::marker::PhantomData,
        }
    }

    fn key(&self, key1: &K1, key2: &K2) -> Vec<u8> {
        let key1_bytes = bincode::serialize(key1).unwrap_or_default();
        let key2_bytes = bincode::serialize(key2).unwrap_or_default();
        mapping_key_2(self.prefix, &key1_bytes, &key2_bytes)
    }

    pub fn get(&self, key1: &K1, key2: &K2) -> Option<V> {
        read(&self.key(key1, key2))
    }

    pub fn get_or_default(&self, key1: &K1, key2: &K2) -> V where V: Default {
        self.get(key1, key2).unwrap_or_default()
    }

    pub fn set(&self, key1: &K1, key2: &K2, value: &V) {
        write(&self.key(key1, key2), value);
    }

    pub fn remove(&self, key1: &K1, key2: &K2) {
        remove(&self.key(key1, key2));
    }
}
