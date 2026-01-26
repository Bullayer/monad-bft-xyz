use crate::{Snapshot, Storage};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum Change {
    Put(Vec<u8>),
    Delete,
}

pub struct Fork<'a> {
    storage: &'a dyn Storage,
    snapshot: Box<dyn Snapshot>,
    changes: HashMap<String, Change>,
}

impl<'a> Fork<'a> {
    pub fn new(storage: &'a dyn Storage) -> Self {
        Self {
            storage,
            snapshot: storage.snapshot(),
            changes: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        match self.changes.get(key) {
            Some(Change::Put(value)) => Some(value.clone()),
            Some(Change::Delete) => None,
            None => self.snapshot.get(key),
        }
    }

    pub fn put(&mut self, key: &str, value: Vec<u8>) {
        self.changes.insert(key.to_string(), Change::Put(value));
    }

    pub fn delete(&mut self, key: &str) {
        self.changes.insert(key.to_string(), Change::Delete);
    }

    pub fn into_patch(self) -> Patch {
        Patch {
            changes: self.changes,
        }
    }

    pub fn rollback(&mut self) {
        self.changes.clear();
    }
}

pub struct Patch {
    changes: HashMap<String, Change>,
}

impl Patch {
    pub fn apply(self, storage: &mut dyn Storage) {
        for (key, change) in self.changes {
            match change {
                Change::Put(value) => storage.put(&key, value),
                Change::Delete => storage.delete(&key),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;

    #[test]
    fn test_fork_put_get() {
        let mut storage = MemoryStore::new();
        storage.put("key1", b"value1".to_vec());

        let mut fork = Fork::new(&storage);
        assert_eq!(fork.get("key1"), Some(b"value1".to_vec()));

        fork.put("key1", b"value2".to_vec());
        fork.put("key2", b"value3".to_vec());

        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("key1"), Some(b"value1".to_vec()));

        assert_eq!(fork.get("key1"), Some(b"value2".to_vec()));
        assert_eq!(fork.get("key2"), Some(b"value3".to_vec()));

        let patch = fork.into_patch();
        patch.apply(&mut storage);

        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("key1"), Some(b"value2".to_vec()));
    }
}