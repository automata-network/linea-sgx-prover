use std::prelude::v1::*;

use base::format::debug;
use eth_types::{
    Block, BlockHeader, HexBytes, Receipt, Signer, TransactionAccessTuple, TransactionInner,
    Withdrawal, SH160, SU256, SU64,
};
use evm_executor::{ExecuteResult, PrecompileSet, TxContext};
use statedb::StateDB;
use statedb::StateFetcher;
use std::sync::Arc;

use mpt::Database;

#[derive(Debug, Clone)]
pub struct Linea {
    chain_id: SU256,
}

impl Linea {
    pub fn new(chain_id: SU256) -> Self {
        Self { chain_id }
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

type Trie<F> = mpt::TrieState<F, Database>;

impl evm_executor::Engine for Linea {
    type Block = Block;
    type BlockHeader = BlockHeader;
    type Receipt = Receipt;
    type Transaction = TransactionInner;
    type Withdrawal = Withdrawal;
    type NewBlockContext = ();

    fn new_block_header(
        &self,
        prev_header: &Self::BlockHeader,
        ctx: Self::NewBlockContext,
    ) -> Self::BlockHeader {
        Self::BlockHeader {
            ..prev_header.clone()
        }
    }

    fn build_receipt(
        &self,
        cumulative_gas_used: u64,
        result: &ExecuteResult,
        tx_idx: usize,
        tx: &Self::Transaction,
        header: &Self::BlockHeader,
    ) -> Self::Receipt {
        let mut receipt = Receipt {
            status: (result.success as u64).into(),
            transaction_hash: tx.hash(),
            transaction_index: (tx_idx as u64).into(),
            r#type: Some(tx.ty().into()),
            gas_used: result.used_gas.into(),
            cumulative_gas_used: (cumulative_gas_used + result.used_gas).into(),
            logs: result.logs.clone(),
            logs_bloom: HexBytes::new(),

            // not affect the rlp encoding
            contract_address: None,
            root: None,
            block_hash: None,
            block_number: None,
        };
        receipt.logs_bloom = eth_types::create_bloom([&receipt].into_iter()).to_hex();
        receipt
    }

    //Both Linea mainnet and testnet are in LONDON fork since the genesis block
    fn evm_config(&self) -> evm::Config {
        let mut cfg = evm::Config::london();
        cfg
    }

    fn precompile(&self) -> PrecompileSet {
        PrecompileSet::berlin()
    }

    fn signer(&self) -> Signer {
        Signer::new(self.chain_id)
    }

    fn process_withdrawals<D: StateDB>(
        &mut self,
        statedb: &mut D,
        withdrawals: &[Self::Withdrawal],
    ) -> Result<(), statedb::Error> {
        Ok(())
    }

    fn author(&self, header: &Self::BlockHeader) -> Result<Option<SH160>, String> {
        let extra_data = header.extra_data.as_bytes();
        let mut sig_array = [0_u8; 65];
        sig_array.copy_from_slice(&extra_data[extra_data.len() - 65..]);
        let msg = Self::seal_hash(header);
        let pub_key_array =
            crypto::secp256k1_ecdsa_recover(&sig_array, &msg).ok_or("fail to ecrecover")?;
        let pub_key = crypto::Secp256k1PublicKey::from_raw_bytes(&pub_key_array);
        let author = pub_key.eth_accountid().into();

        Ok(Some(author))
    }

    fn tx_context<'a>(&self, ctx: &mut TxContext<'a, Self::Transaction, Self::BlockHeader>) {
        glog::debug!("Set base fee: {:?}", ctx.header.base_fee_per_gas);
        ctx.block_base_fee = ctx.header.base_fee_per_gas;
        ctx.difficulty = ctx.header.difficulty;
    }

    fn finalize_block<D: StateDB>(
        &mut self,
        statedb: &mut D,
        mut header: Self::BlockHeader,
        txs: Vec<Arc<Self::Transaction>>,
        receipts: Vec<Self::Receipt>,
        withdrawals: Option<Vec<Self::Withdrawal>>,
    ) -> Result<Self::Block, String> {
        Ok(Block::new(header, txs, &receipts, withdrawals))
    }
}
