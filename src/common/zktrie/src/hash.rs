use std::prelude::v1::*;

use eth_types::SH256;

use crate::Error;

pub fn trie_hash(data: &[u8]) -> Result<SH256, Error> {
    mimc::sum(data).map(|n| n.into()).map_err(Error::HashFail)
}
