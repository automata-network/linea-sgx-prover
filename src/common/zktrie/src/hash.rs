use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::SH256;

use crate::Error;
lazy_static::lazy_static!{
    pub static ref EMPTY_KECCAK_CODE_HASH: SH256 = keccak_hash(&[]).into();
    pub static ref EMPTY_MIMC_CODE_HASH: SH256 = trie_hash(SH256::default().as_bytes()).unwrap();
}

pub fn trie_hash(data: &[u8]) -> Result<SH256, Error> {
    mimc::sum(data).map(|n| n.into()).map_err(Error::HashFail)
}

