use std::prelude::v1::*;

use eth_types::{BlockHeader, BlockNonce, Bloom, Nilable, SH160, SH256, SU256, HexBytes};
use rlp_derive::RlpEncodable;

pub struct Linea {}

impl Linea {
    pub fn author(&self, header: &BlockHeader) -> SH160 {
        let extra_data = header.extra_data.as_bytes();
        let mut sig_array = [0_u8; 65];
        sig_array.copy_from_slice(&extra_data[extra_data.len() - 65..]);
        let msg = Linea::seal_hash(header);
        let pub_key_array = crypto::secp256k1_ecdsa_recover(&sig_array, &msg).unwrap();
        let pub_key = crypto::Secp256k1PublicKey::from_raw_bytes(&pub_key_array);
        let author = pub_key.eth_accountid().into();
        glog::info!("author: {:?}", author);
        return author;
    }

    fn seal_hash(header: &BlockHeader) -> [u8; 32] {
        // Remove the last 65 bytes of extra_data
        let extra_data: HexBytes = header.extra_data[..header.extra_data.len() - 65].into();
        // May need to handle the case where base_fee_per_gas is nil
        // https://github.com/ethereum/go-ethereum/blob/81fd1b3cf9c4c4c9f0e06f8bdcbaa8b29c81b052/consensus/clique/clique.go#L763
        let mut s = rlp::RlpStream::new_list(16);
        s.append(&header.parent_hash);
        s.append(&header.sha3_uncles);
        s.append(&header.miner);
        s.append(&header.state_root);
        s.append(&header.transactions_root);
        s.append(&header.receipts_root);
        s.append(&header.logs_bloom);
        s.append(&header.difficulty);
        s.append(&header.number);
        s.append(&header.gas_limit);
        s.append(&header.gas_used);
        s.append(&header.timestamp);
        s.append(&extra_data);
        s.append(&header.mix_hash);
        s.append(&header.nonce);
        s.append(&header.base_fee_per_gas);
        
        let data = s.out().to_vec();
        return crypto::keccak_hash(&data);
    }
}
