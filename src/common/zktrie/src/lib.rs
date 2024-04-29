#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(feature = "tstd")]
#[macro_use]
extern crate sgxlib as std;

use std::prelude::v1::*;

mod trie;
use eth_types::{SH256, HexBytes};
pub use trie::*;

mod storage;
pub use storage::*;

mod trace;
pub use trace::*;

mod node;
pub use node::*;

mod hash;
pub use hash::*;

mod sparse_merkle_trie;
pub use sparse_merkle_trie::*;

mod utils;
pub use utils::*;

#[derive(Debug, PartialEq)]
pub enum Error {
    // KeyNotFound,
    ReachedMaxLevel,
    PathNotAllow,
    InMemNextNodeNotFound,
    NodeNotFound(usize, SH256),
    ZKTrieKeyNotFound(HexBytes),
    ZkTrieParseNodeFail(HexBytes, &'static str),
    HashFail(String),
    InvalidBranchNode(HexBytes),
    InvalidProof,
}