use crate::{Snapshot, Storage};
use std::marker::PhantomData;

pub trait Binary: Sized {
    fn to_bytes(&self) -> Vec<u8>;
    fn from_bytes(bytes: &[u8]) -> Result<Self, String>;
}

impl Binary for u32 {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 4 {
            return Err("Invalid byte length for u32".to_string());
        }
        let mut array = [0u8; 4];
        array.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(array))
    }
}

impl Binary for String {
    fn to_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        String::from_utf8(bytes.to_vec())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

pub struct ListIndex<'a, T> {
    snapshot: &'a dyn Snapshot,
    prefix: String,
    _phantom: PhantomData<T>,
}

impl<'a, T: Binary> ListIndex<'a, T> {
    pub fn new(snapshot: &'a dyn Snapshot, prefix: &str) -> Self {
        Self {
            snapshot,
            prefix: prefix.to_string(),
            _phantom: PhantomData,
        }
    }

    pub fn get(&self, index: usize) -> Option<T> {
        let key = format!("{}:{}", self.prefix, index);
        self.snapshot.get(&key)
            .and_then(|bytes| T::from_bytes(&bytes).ok())
    }

    pub fn len(&self) -> usize {
        let len_key = format!("{}:len", self.prefix);
        self.snapshot.get(&len_key)
            .and_then(|bytes| u32::from_bytes(&bytes).ok())
            .map(|len| len as usize)
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct ListIndexMut<'a, T> {
    storage: &'a mut dyn Storage,
    prefix: String,
    _phantom: PhantomData<T>,
    changes: Vec<(usize, Option<Vec<u8>>)>,
    len: usize,
}

impl<'a, T: Binary> ListIndexMut<'a, T> {
    pub fn new(storage: &'a mut dyn Storage, prefix: &str) -> Self {
        let prefix = prefix.to_string();
        let len_key = format!("{}:len", prefix);
        let len = storage.snapshot().get(&len_key)
            .and_then(|bytes| u32::from_bytes(&bytes).ok())
            .map(|len| len as usize)
            .unwrap_or(0);

        Self {
            storage,
            prefix,
            _phantom: PhantomData,
            changes: Vec::new(),
            len,
        }
    }

    pub fn push(&mut self, value: T) {
        let index = self.len;
        self.changes.push((index, Some(value.to_bytes())));
        self.len += 1;
    }

    pub fn get(&self, index: usize) -> Option<T> {
        for (idx, value_opt) in &self.changes {
            if *idx == index {
                return value_opt.as_ref().and_then(|bytes| T::from_bytes(bytes).ok());
            }
        }
        let key = format!("{}:{}", self.prefix, index);
        self.storage.snapshot().get(&key)
            .and_then(|bytes| T::from_bytes(&bytes).ok())
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn commit(&mut self) {
        for (index, value_opt) in self.changes.drain(..) {
            let key = format!("{}:{}", self.prefix, index);
            if let Some(value) = value_opt {
                self.storage.put(&key, value);
            } else {
                self.storage.delete(&key);
            }
        }
        
        let len_key = format!("{}:len", self.prefix);
        self.storage.put(&len_key, (self.len as u32).to_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;

    #[test]
    fn test_list_index() {
        let mut storage = MemoryStore::new();
        
        {
            let mut list = ListIndexMut::new(&mut storage, "my_list");
            list.push(42u32);
            list.push(100u32);
            list.commit();
        }

        let snapshot = storage.snapshot();
        let list = ListIndex::new(&*snapshot, "my_list");

        println!("{:?}", list.get(0));
        println!("{:?}", list.get(1));
        
        assert_eq!(list.len(), 2);
        assert_eq!(list.get(0), Some(42u32));
        assert_eq!(list.get(1), Some(100u32));
    }

    #[test]
    fn text_string_list_index() {
        let mut storage = MemoryStore::new();
        
        {
            let mut list = ListIndexMut::new(&mut storage, "str_list");
            list.push("hello".to_string());
            list.push("world".to_string());
            list.commit();
        }

        let snapshot = storage.snapshot();
        let list = ListIndex::new(&*snapshot, "str_list");

        println!("{:?}", list.get(0));
        println!("{:?}", list.get(1));
        
        assert_eq!(list.len(), 2);
        assert_eq!(list.get(0), Some("hello".to_string()));
        assert_eq!(list.get(1), Some("world".to_string()));
    }
}

