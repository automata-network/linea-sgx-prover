use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::{Hasher, HexBytes, SH256};
use mpt::{TrieNode, TrieStorageNode};
use statedb::NodeDB;
use std::borrow::Cow;
use std::sync::Arc;

pub struct Database {
    db: statedb::MemStore<TrieStorageNode, NodeHasher>,
}

pub struct NodeHasher;
impl statedb::Hasher<TrieStorageNode> for NodeHasher {
    fn hash(n: &TrieStorageNode) -> SH256 {
        n.hash
    }
}

impl Database {
    pub fn new(cap: usize) -> Self {
        Self {
            db: statedb::MemStore::new(cap),
        }
    }

    pub fn resume_code(&mut self, code: &[u8]) {
        let hash = keccak_hash(code).into();
        self.db.set_code(hash, Cow::Owned(code.into()));
    }

    pub fn resume_node(&mut self, node: &[u8]) -> Option<Arc<TrieStorageNode>> {
        match rlp::decode(&node) {
            Ok(n) => match n {
                TrieNode::Embedded(n) => {
                    self.add_node(&n);
                    return Some(n);
                }
                TrieNode::Nil => {}
                TrieNode::Hash(_) => unreachable!(),
            },
            Err(err) => {
                glog::error!("decode proof string fail: {:?}", err);
            }
        }
        None
    }

    pub fn resume_proofs(&mut self, proofs: &[HexBytes]) -> Option<Arc<TrieStorageNode>> {
        let mut leaf = None;
        for proof in proofs {
            leaf = self.resume_node(&proof);
        }
        leaf
    }
}

impl statedb::NodeDB for Database {
    type Node = TrieStorageNode;

    fn add_node(&mut self, node: &Arc<Self::Node>) {
        self.db.add_node(node)
    }

    fn commit(&mut self) -> usize {
        self.db.commit()
    }

    fn fork(&self) -> Self {
        Self { db: self.db.fork() }
    }

    fn get(&self, index: &SH256) -> Option<Arc<Self::Node>> {
        self.db.get(index)
    }

    fn get_code(&mut self, hash: &SH256) -> Option<Arc<HexBytes>> {
        self.db.get_code(hash)
    }

    fn remove_staging_node(&mut self, node: &Arc<Self::Node>) {
        self.db.remove_staging_node(node)
    }

    fn set_code(&mut self, hash: SH256, code: Cow<HexBytes>) {
        self.db.set_code(hash, code)
    }

    fn staging(&mut self, node: Self::Node) -> Arc<Self::Node> {
        self.db.staging(node)
    }
}
