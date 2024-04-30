use std::prelude::v1::*;

use base::format::debug;
use eth_types::{SH256, HexBytes};
use statedb::TrieUpdate;
use zktrie::{Database, Node};

use crate::ZkStateAccount;

pub trait Trie: Sized {
    type DB: Database;
    fn root_hash(&self) -> SH256;
    fn try_get(&self, db: &mut Self::DB, key: &[u8]) -> Option<Vec<u8>>;
    fn get(&self, db: &mut Self::DB, key: &[u8]) -> Result<Vec<u8>, String>;
    fn update(&mut self, db: &mut Self::DB, updates: Vec<(&[u8], Vec<u8>)>) -> Vec<TrieUpdate>;
    fn new_root(&self, new_root: SH256) -> Self;
}

#[derive(Debug)]
pub struct ZkTrie<D: Database<Node = Node>> {
    raw: zktrie::ZkTrie<D>,
}

impl<D: Database<Node = Node>> ZkTrie<D> {
    pub fn new(root: SH256) -> Self {
        let t = <zktrie::ZkTrie<D>>::new(root);
        Self { raw: t }
    }
}

impl<D: Database<Node = Node>> Trie for ZkTrie<D> {
    type DB = D;
    fn get(&self, db: &mut Self::DB, key: &[u8]) -> Result<Vec<u8>, String> {
        let val = self.raw.read(db, key).map_err(debug)?.unwrap_or_default();
        Ok(val)
    }

    fn new_root(&self, new_root: SH256) -> Self {
        let raw = zktrie::ZkTrie::new(new_root);
        Self { raw }
    }

    fn root_hash(&self) -> SH256 {
        *self.raw.top_root_hash()
    }

    fn try_get(&self, db: &mut Self::DB, key: &[u8]) -> Option<Vec<u8>> {
        unimplemented!()
    }

    fn update(&mut self, db: &mut Self::DB, mut updates: Vec<(&[u8], Vec<u8>)>) -> Vec<TrieUpdate> {
        updates.sort_by(|(a, _), (b, _)| b.cmp(a));
        let mut result = Vec::new();
        for (item, data) in updates {
            if data.len() == 192 {
                let acc = ZkStateAccount::from_bytes(&data);
                glog::info!("acc: {:?}", acc);
            }

            glog::info!("updates: {:?} -> {:?}", HexBytes::from(item), HexBytes::from(data.as_slice()));
            if data.len() == 0 {
                self.raw.remove(db, item).unwrap();
            } else {
                self.raw.put(db, item, data).unwrap();
            }
    result.push(TrieUpdate::Success);
        }
        result
    }
}