use std::prelude::v1::*;

use base::format::debug;
use eth_types::SH256;
use statedb::TrieUpdate;
use zktrie::{Database, Node};

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
    pub fn new(next_free_node: u64, sub_root: SH256) -> Self {
        let t = <zktrie::ZkTrie<D>>::new_from_sub_root(next_free_node, sub_root);
        Self { raw: t }
    }
}

impl<D: Database<Node = Node>> Trie for ZkTrie<D> {
    type DB = D;
    fn get(&self, db: &mut Self::DB, key: &[u8]) -> Result<Vec<u8>, String> {
        let val = self.raw.read(db, key).map_err(debug)?.unwrap_or_default();
        Ok(val)
    }

    fn new_root(&self, new_root: SH256) -> Result<Self, String> {
        let raw = zktrie::ZkTrie::new(db, new_root).map_err(debug)?;
        Ok(Self { raw })
    }

    fn root_hash(&self) -> SH256 {
        unimplemented!()
    }

    fn try_get(&self, db: &mut Self::DB, key: &[u8]) -> Option<Vec<u8>> {
        unimplemented!()
    }

    fn update(&mut self, db: &mut Self::DB, updates: Vec<(&[u8], Vec<u8>)>) -> Vec<TrieUpdate> {
        unimplemented!()
    }
}
