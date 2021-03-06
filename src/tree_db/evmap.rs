use std::collections::HashMap;
use std::path::PathBuf;

use crate::constants::KEY_LEN;
use crate::traits::{Database, Exception};
use crate::tree::tree_node::TreeNode;

use evmap::{ReadHandle, WriteHandle};
use parking_lot::Mutex;

pub struct HashDB {
    read: ReadHandle<[u8; KEY_LEN], TreeNode>,
    write: Mutex<WriteHandle<[u8; KEY_LEN], TreeNode>>,
}

impl HashDB {
    #[inline]
    pub fn new(_: HashMap<[u8; KEY_LEN], TreeNode>) -> Self {
        let (read, write) = evmap::new();
        Self {
            read,
            write: Mutex::new(write),
        }
    }
}

impl Database for HashDB {
    type NodeType = TreeNode;
    type EntryType = ([u8; KEY_LEN], TreeNode);

    #[inline]
    fn open(_path: &PathBuf) -> Result<Self, Exception> {
        Ok(Self::new(HashMap::new()))
    }

    #[inline]
    fn get_node(&self, key: &[u8; KEY_LEN]) -> Result<Option<Self::NodeType>, Exception> {
        if let Some(m) = self.read.get_and(key, |x| x[x.len() - 1].clone()) {
            return Ok(Some(m));
        } else {
            return Ok(None);
        }
    }

    #[inline]
    fn insert(&mut self, key: [u8; KEY_LEN], value: Self::NodeType) -> Result<(), Exception> {
        self.write.lock().insert(key, value);
        Ok(())
    }

    #[inline]
    fn remove(&mut self, key: &[u8; KEY_LEN]) -> Result<(), Exception> {
        self.write.lock().empty(*key);
        self.write.lock().refresh();
        Ok(())
    }

    #[inline]
    fn batch_write(&mut self) -> Result<(), Exception> {
        self.write.lock().refresh();
        Ok(())
    }
}

unsafe impl Sync for HashDB {}
unsafe impl Send for HashDB {}
