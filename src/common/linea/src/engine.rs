use std::prelude::v1::*;

use eth_types::{BlockHeader, BlockNonce, Bloom, Nilable, SH160, SH256, SU256};
use rlp_derive::RlpEncodable;

pub struct Linea {}

impl Linea {
    pub fn author(&self, header: &BlockHeader) -> SH160 {
        "0x8F81e2E3F8b46467523463835F965fFE476E1c9E".into()
    }

    fn encode_header(header: &BlockHeader) -> Vec<u8> {
        #[derive(RlpEncodable)]
        struct Header {
            // parent_hash: &'a SH256,
            // uncle_hash: &'a SH256,
            // coinbase: &'a SH160,
            // root: &'a SH256,
            // tx_hash: &'a SH256,
            // receipt_hash: &'a SH256,
            // bloom: &'a [u8],
            // difficulty: &'a SU256,
            // number: SU256,
            // gas_limit: u64,
            // gas_used: u64,
            // time: u64,
            // extra: &'a [u8],
            // mix_digest: &'a SH256,
            nonce: BlockNonce,
            base_fee: Nilable<SU256>,
        }
        let encode = Header {
            // parent_hash: &header.parent_hash,
            // uncle_hash: &header.sha3_uncles,
            // coinbase: &header.miner,
            // root: &header.state_root,
            // tx_hash: &header.transactions_root,
            // receipt_hash: &header.receipts_root,
            // bloom: &header.logs_bloom,
            // difficulty: &header.difficulty,
            // number: header.number.as_u256(),
            // gas_limit: header.gas_limit.as_u64(),
            // gas_used: header.gas_used.as_u64(),
            // time: header.timestamp.as_u64(),
            // extra: &header.extra_data[..header.extra_data.len() - 65],
            // mix_digest: &header.mix_hash,
            nonce: header.nonce.clone(),
            base_fee: Some(header.base_fee_per_gas).into(),
        };
        rlp::encode(&encode).to_vec()
    }
}
