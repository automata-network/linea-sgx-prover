use std::prelude::v1::*;

use crate::Database;
use base::format::parse_ether;
use eth_tools::Pob;
use eth_types::{Transaction, TransactionInner, SU256, BlockHeader};
use executor::ExecuteError;
use mpt::TrieState;
use statedb::NodeDB;
use std::sync::Arc;

pub struct BlockExecutor {}

impl BlockExecutor {
    pub fn new() -> Self {
        Self {}
    }

    pub fn execute(&self, db: &Database, pob: Pob) -> Result<(), String> {
        let mut db = db.fork();
        for node in &pob.data.mpt_nodes {
            db.resume_node(node);
        }
        for code in &pob.data.codes {
            db.resume_code(&code);
        }
        db.commit();

        let parent = Arc::new(pob.block.header.clone());
        let state = TrieState::new((), parent, db);
        let txs = self.preprocess_txs(pob.block.transactions)?;
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
