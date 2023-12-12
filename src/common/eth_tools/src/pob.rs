use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::{Block, FetchStateResult, HexBytes, SH256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
pub struct Pob {
    pub block: Block,
    pub data: PobData,
}

impl Pob {
    pub fn new(block: Block, mut data: PobData) -> Pob {
        data.mpt_nodes.sort_unstable();
        Pob { block, data }
    }

    pub fn from_proof(
        chain_id: u64,
        blk: Block,
        prev_state_root: SH256,
        block_hashes: BTreeMap<u64, SH256>,
        codes: BTreeMap<SH256, HexBytes>,
        states: Vec<FetchStateResult>,
    ) -> Pob {
        let codes = codes.into_values().collect();
        let mut mpt_nodes = BTreeMap::new();
        for state in states {
            if let Some(acc) = state.acc {
                for node in acc.account_proof {
                    let hash: SH256 = keccak_hash(&node).into();
                    mpt_nodes.entry(hash).or_insert(node);
                }
                for storage in acc.storage_proof {
                    for node in storage.proof {
                        let hash: SH256 = keccak_hash(&node).into();
                        mpt_nodes.entry(hash).or_insert(node);
                    }
                }
            }
        }
        let mpt_nodes = mpt_nodes.into_values().collect();
        let data = PobData {
            chain_id,
            prev_state_root,
            block_hashes,
            mpt_nodes,
            codes,
        };
        Pob { block: blk, data }
    }

    pub fn state_hash(&self) -> SH256 {
        // the mpt_nodes should be in order
        crypto::keccak_encode(|hash| {
            for item in &self.data.mpt_nodes {
                hash(&item);
            }
        })
        .into()
    }

    pub fn block_hash(&self) -> SH256 {
        self.block.header.hash()
    }
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct PobData {
    pub chain_id: u64,
    pub prev_state_root: SH256,
    pub block_hashes: BTreeMap<u64, SH256>,
    pub mpt_nodes: Vec<HexBytes>,
    pub codes: Vec<HexBytes>,
}
