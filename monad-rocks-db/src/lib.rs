use std::collections::HashMap;

pub mod index;
pub mod fork;
pub mod rocksdb_store;

pub use index::{ListIndex, ListIndexMut, Binary};
pub use rocksdb_store::{RocksDBStore, RocksDBConfig, ColumnFamilyConfig, RocksDBStoreError, RocksDBFork, RocksDBPatch};
pub use fork::{Fork, Patch};

pub trait Snapshot {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn contains(&self, key: &str) -> bool;
}

pub trait Storage {
    fn snapshot(&self) -> Box<dyn Snapshot>;

    fn put(&mut self, key: &str, value: Vec<u8>);

    fn delete(&mut self, key: &str);
}

pub struct MemoryStore {
    store: HashMap<String, Vec<u8>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot for MemoryStore {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store.get(key).cloned()
    }

    fn contains(&self, key: &str) -> bool {
        self.store.contains_key(key)
    }
}

impl Storage for MemoryStore {
    fn snapshot(&self) -> Box<dyn Snapshot> {
        Box::new(MemoryStore {
            store: self.store.clone(),
        })
    }

    fn put(&mut self, key: &str, value: Vec<u8>) {
        self.store.insert(key.to_string(), value);
    }

    fn delete(&mut self, key: &str) {
        self.store.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot() {
        let mut storage = MemoryStore::new();
        storage.put("key1", b"value1".to_vec());
        
        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("key1"), Some(b"value1".to_vec()));
        
        storage.put("key1", b"value2".to_vec());
        
        assert_eq!(snapshot.get("key1"), Some(b"value1".to_vec()));
    
        let new_snapshot = storage.snapshot();
        assert_eq!(new_snapshot.get("key1"), Some(b"value2".to_vec()));
    }
}
