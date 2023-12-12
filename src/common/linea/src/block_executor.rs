use std::prelude::v1::*;

use crate::Database;
use base::format::parse_ether;
use eth_tools::Pob;
use eth_types::{BlockHeader, PoolTx, Signer, Transaction, TransactionInner, SU256};
use executor::{Context, ExecuteError, PrecompileSet};
use mpt::TrieState;
use statedb::{NodeDB, StateDB};
use std::sync::Arc;

pub struct BlockExecutor {
    signer: Signer,
}

impl BlockExecutor {
    pub fn new(chain_id: SU256) -> Self {
        let signer = Signer::new(chain_id);
        Self { signer }
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
                cost_gas_fee: true,
                gas_overcommit: false,
            };
            let result = executor::TxExecutor::new(ctx, &mut state).execute();
            glog::info!("result{:?}", result);
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
