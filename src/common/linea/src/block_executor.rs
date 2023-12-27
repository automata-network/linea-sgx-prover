use std::prelude::v1::*;

use crate::Linea;
use base::format::{debug, parse_ether};
use eth_tools::{ExecutionClient, MixRpcClient, Pob};
use eth_types::{
    BlockHeader, BlockSelector, Signer, Transaction, TransactionInner, TxTrait, SU256,
};
use evm_executor::{BlockBuilder, Engine, ExecuteError, PrecompileSet, TxContext};
use mpt::{BlockStateFetcher, Database, ReductionNodeFetcher, TrieState};
use statedb::NodeDB;
use std::sync::Arc;

pub struct BlockExecutor {
    engine: Linea,
}

impl BlockExecutor {
    pub fn new(chain_id: SU256) -> Self {
        Self {
            engine: Linea::new(chain_id),
        }
    }

    pub fn generate_pob(
        &self,
        client: &ExecutionClient<Arc<MixRpcClient>>,
        block: BlockSelector,
    ) -> Result<Pob, String> {
        let chain_id = self.engine.signer().chain_id;
        let mut pob = client
            .generate_pob(chain_id.as_u64(), block)
            .map_err(debug)?;

        let mut db = Database::new(100000);
        self.resume_db(&mut pob, &mut db);

        if true {
            // fill reduction node
            let header = pob.block.header.clone();
            let mut fetcher = ReductionNodeFetcher::new(client);
            let statedb = mpt::TrieState::new(fetcher.clone(), pob.data.prev_state_root, db);
            let mut builder = BlockBuilder::new(self.engine.clone(), statedb, (), header).unwrap();
            let txs = self.preprocess_txs(pob.block.transactions.clone())?;
            for tx in txs {
                builder.commit(Arc::new(tx)).map_err(debug)?;
            }
            builder.flush_state().map_err(debug)?;
            for (_, node) in fetcher.take() {
                pob.data.mpt_nodes.push(node);
            }
        }
        Ok(pob)
    }

    fn resume_db(&self, pob: &Pob, db: &mut Database) {
        for node in &pob.data.mpt_nodes {
            db.resume_node(node);
        }
        for code in &pob.data.codes {
            db.resume_code(&code);
        }
        db.commit();
    }

    pub fn execute(&self, db: &Database, pob: Pob) -> Result<(), String> {
        if pob.data.chain_id != self.engine.signer().chain_id.as_u64() {
            return Err(format!(
                "chain_id mismatch {}!={}",
                pob.data.chain_id,
                self.engine.signer().chain_id
            ));
        }

        let mut db = db.fork();
        self.resume_db(&pob, &mut db);

        let expect_root = pob.block.header.state_root;
        let number = pob.block.header.number.as_u64();
        let mut header = pob.block.header;

        let statedb = mpt::TrieState::new((), pob.data.prev_state_root, db);
        let mut builder = BlockBuilder::new(self.engine.clone(), statedb, (), header.clone()).unwrap();
        let txs = self.preprocess_txs(pob.block.transactions)?;
        let total = txs.len();
        for (idx, tx) in txs.into_iter().enumerate() {
            // glog::info!("[{}/{}]tx: {:?}", idx, total, tx.hash());
            let result = builder.commit(Arc::new(tx)).unwrap();
            glog::debug!("Txn execute result: {:?}", result);
        }
        if let Some(withdrawals) = pob.block.withdrawals {
            builder.withdrawal(withdrawals).unwrap();
        }
        let block = builder.finalize().unwrap();
        let new_state = block.header.state_root;
        if (new_state != expect_root) {
            glog::error!("Block#{:?}, root mismatch: {:?} != {:?}", number, new_state, expect_root);
            panic!("DIE, block#{:?} mismatch", number);
        } else {
            if (number % 10 == 0) {
                glog::info!("Block#{:?}, root match: {:?}", number, new_state);
            }
        }
        // assert!(
        //     new_state == expect_root,
        //     "block: {}, want: {:?}, got: {:?}, begin: {:?}",
        //     number,
        //     expect_root,
        //     new_state,
        //     pob.data.prev_state_root,
        // );
        // glog::info!("root: {:?} vs {:?}", new_state, expect_root);
        return Ok(());
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
