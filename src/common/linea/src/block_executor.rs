use std::{prelude::v1::*, sync::Mutex};

use crate::Linea;
use base::format::{debug, parse_ether};
use eth_tools::{ExecutionClient, MixRpcClient, Pob, RpcClient};
use eth_types::{
    Block, BlockHeader, BlockSelector, HexBytes, Receipt, Signer, Transaction, TransactionInner,
    TxTrait, SH256, SU256, SU64,
};
use evm_executor::{BlockBuilder, BlockHashGetter, Engine, ExecuteError, PrecompileSet, TxContext};
use mpt::{BlockStateFetcher, Database, StateCollector, TrieState};
use statedb::{NoStateFetcher, NodeDB};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct BlockExecutor {
    engine: Linea,
}

pub struct BlockHashCache {
    cache: BTreeMap<u64, SH256>,
}

impl BlockHashCache {
    pub fn new(cache: BTreeMap<u64, SH256>) -> Self {
        Self { cache }
    }
}

impl BlockHashGetter for BlockHashCache {
    fn get_hash(&self, current: u64, target: u64) -> SH256 {
        if target >= current || target < current.saturating_sub(256) {
            return Default::default();
        }
        self.cache.get(&target).cloned().unwrap_or_default()
    }
}

#[derive(Clone)]
pub struct BuilderFetcher<C: RpcClient> {
    client: ExecutionClient<C>,
    cache: Arc<Mutex<BTreeMap<u64, SH256>>>,
}

impl<C: RpcClient> BuilderFetcher<C> {
    pub fn new(client: ExecutionClient<C>) -> Self {
        Self {
            client,
            cache: Default::default(),
        }
    }
}

impl<C: RpcClient> BlockHashGetter for BuilderFetcher<C> {
    fn get_hash(&self, current: u64, target: u64) -> SH256 {
        if target >= current || target < current.saturating_sub(256) {
            return Default::default();
        }
        {
            let cache = self.cache.lock().unwrap();
            if let Some(hash) = cache.get(&target) {
                return *hash;
            }
        }
        match self.client.get_block_header(target.into()) {
            Ok(header) => {
                let hash = header.hash();
                let mut cache = self.cache.lock().unwrap();
                cache.insert(target, hash);
                hash
            }
            Err(err) => Default::default(),
        }
    }
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

        let builder_fetcher = BuilderFetcher::new(client.clone());

        if true {
            // fill reduction node
            let header = pob.block.header.clone();
            let mut fetcher =
                StateCollector::new(client, (pob.block.header.number - SU64::from(1)).into());
            let statedb = mpt::TrieState::new(fetcher.clone(), pob.data.prev_state_root, db);
            let mut builder = BlockBuilder::new(
                self.engine.clone(),
                statedb,
                builder_fetcher.clone(),
                header,
            )
            .unwrap();
            let txs = self.preprocess_txs(pob.block.transactions.clone())?;
            for tx in txs {
                builder.commit(Arc::new(tx)).map_err(debug)?;
            }
            builder.flush_state().map_err(debug)?;
            for (_, node) in fetcher.take() {
                pob.data.mpt_nodes.push(node);
            }
            {
                let cache = builder_fetcher.cache.lock().unwrap();
                for (block_no, block_hash) in cache.iter() {
                    pob.data.block_hashes.insert(*block_no, *block_hash);
                }
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

    pub fn execute(
        &self,
        client: &ExecutionClient<Arc<MixRpcClient>>,
        db: &Database,
        pob: Pob,
    ) -> Result<Block, String> {
        if pob.data.chain_id != self.engine.signer().chain_id.as_u64() {
            return Err(format!(
                "chain_id mismatch {}!={}",
                pob.data.chain_id,
                self.engine.signer().chain_id
            ));
        }

        let mut db = db.fork();
        self.resume_db(&pob, &mut db);

        let builder_fetcher = BlockHashCache::new(pob.data.block_hashes);

        let number = pob.block.header.number.as_u64();
        let mut header = pob.block.header;

        let statedb = mpt::TrieState::new((), pob.data.prev_state_root, db);
        let mut builder = BlockBuilder::new(
            self.engine.clone(),
            statedb,
            builder_fetcher,
            header.clone(),
        )
        .unwrap();
        let txs = self.preprocess_txs(pob.block.transactions)?;
        let total = txs.len();
        let blkno = header.number.as_u64();
        for (idx, tx) in txs.into_iter().enumerate() {
            // glog::info!("[{}][{}/{}]tx: {:?}", blkno, idx, total, tx.hash());
            let tx = Arc::new(tx);
            let receipt = builder.commit(tx.clone()).unwrap();
            // let expect_receipt = client.get_receipt(&tx.hash()).unwrap().unwrap();
            // if let Err(err) = Receipt::compare(&expect_receipt, receipt) {
            //     glog::info!("diff: {}", err);
            // }
        }
        if let Some(withdrawals) = pob.block.withdrawals {
            builder.withdrawal(withdrawals).unwrap();
        }
        let block = builder.finalize().unwrap();
        let new_state = block.header.state_root;
        if (new_state != header.state_root) {
            glog::error!("Block#{:?}, root mismatch: {:?} != {:?}", number, new_state, header.state_root);
            // panic!("DIE, block#{:?} mismatch", number);
        } else {
            if (number % 100 == 0) {
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
        return Ok(block);
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
