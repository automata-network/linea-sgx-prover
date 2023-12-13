use std::prelude::v1::*;

use crate::{Database, Linea};
use base::format::parse_ether;
use eth_tools::Pob;
use eth_types::{BlockHeader, PoolTx, Signer, Transaction, TransactionInner, SU256, Bloom, BlockNonce, Nilable, SH256, SH160};
use executor::{Context, ExecuteError, PrecompileSet};
use mpt::TrieState;
use rlp_derive::RlpEncodable;
use statedb::{NodeDB, StateDB};
use std::sync::Arc;

pub struct BlockExecutor {
    signer: Signer,
    engine: Linea,
}

impl BlockExecutor {
    pub fn new(chain_id: SU256) -> Self {
        let signer = Signer::new(chain_id);
        Self { signer, engine: Linea {  } }
    }

    pub fn execute(&self, db: &Database, pob: Pob) -> Result<(), String> {
        if pob.data.chain_id != self.signer.chain_id.as_u64() {
            return Err(format!(
                "chain_id mismatch {}!={}",
                pob.data.chain_id, self.signer.chain_id
            ));
        }

        let mut db = db.fork();
        for node in &pob.data.mpt_nodes {
            db.resume_node(node);
        }
        for code in &pob.data.codes {
            db.resume_code(&code);
        }
        db.commit();

        let parent = Arc::new(pob.block.header.clone());
        let mut state = TrieState::new((), pob.data.prev_state_root, db);
        let txs = self.preprocess_txs(pob.block.transactions)?;
        let cfg = evm::Config::berlin();
        let precompile_set = PrecompileSet::berlin();
        let miner = pob.block.header.miner;
        let miner = self.engine.author(&pob.block.header);
        // let extra_data = pob.block.header.extra_data.as_bytes();
        // let mut sig =  [0_u8;65];
        // sig.copy_from_slice(&extra_data[extra_data.len()-65..]);
        // let sig = crypto::Secp256k1RecoverableSignature::new(sig);
        // crypto::secp256k1_recover_pubkey(&sig, msg)
        // glog::info!("extra_data: {:?}", pob.block.header.extra_data);

        for (tx_idx, tx) in txs.into_iter().enumerate() {
            let caller = tx.sender(&self.signer);
            let tx = PoolTx::with_tx(&self.signer, tx);
            let ctx = Context {
                chain_id: &self.signer.chain_id,
                caller: &caller,
                cfg: &cfg,
                precompile: &precompile_set,
                tx: &tx,
                header: &parent,
                extra_fee: None,
                no_gas_fee: false,
                gas_overcommit: false,
                miner: Some(miner),
            };
            let result = executor::TxExecutor::new(ctx, &mut state).execute();
            glog::info!("Txn execute result: {:?}", result);
        }
        let root = state.flush();

        let root = state.account_trie().hash();
        glog::info!("root: {:?}", root);
        Ok(())
    }

    fn preprocess_txs(&self, txs: Vec<Transaction>) -> Result<Vec<TransactionInner>, String> {
        let mut out = Vec::with_capacity(txs.len());
        for tx in txs {
            let tx = match tx.inner() {
                Some(tx) => tx,
                None => return Err("invalid transaction".into()),
            };
            out.push(tx);
        }

        Ok(out)
    }

    fn effective_gas_tip(
        &self,
        header: &BlockHeader,
        tx: &TransactionInner,
    ) -> Result<SU256, ExecuteError> {
        let base_fee = Some(&header.base_fee_per_gas);
        let zero = SU256::zero();
        match tx.effective_gas_tip(base_fee) {
            Some(n) => Ok(n),
            None => Err(ExecuteError::InsufficientBaseFee {
                tx_hash: tx.hash(),
                block_number: header.number.as_u64().into(),
                block_base_fee_gwei: parse_ether(base_fee.unwrap_or(&zero), 9),
                base_fee_gwei: parse_ether(&tx.effective_gas_tip(None).unwrap(), 9),
            }),
        }
    }
}
