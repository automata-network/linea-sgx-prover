use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::Signer;
use eth_types::{
    BlockSelector, FetchState, FetchStateResult, HexBytes, TransactionAccessTuple, SH160, SH256,
    SU256,
};
use jsonrpc::{MixRpcClient, RpcClient, RpcError};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::Pob;

#[derive(Debug, Clone)]
pub struct ExecutionClient<C: RpcClient = MixRpcClient> {
    client: eth_tools::ExecutionClient<C>,
}

impl<C: RpcClient> std::ops::Deref for ExecutionClient<C> {
    type Target = eth_tools::ExecutionClient<C>;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

#[derive(Debug, Serialize)]
pub struct TraceConfig {
    pub tracer: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxPrestateResult {
    pub tx_hash: SH256,
    pub result: Option<BTreeMap<SH160, PrestateAccount>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrestateAccount {
    pub balance: SU256,
    #[serde(default)]
    pub code: HexBytes,
    #[serde(default)]
    pub nonce: u64,
    #[serde(default)]
    pub storage: BTreeMap<SH256, SH256>,
}

impl<C: RpcClient> ExecutionClient<C> {
    pub fn new(client: C) -> Self {
        Self {
            client: eth_tools::ExecutionClient::new(client),
        }
    }

    pub fn generate_pob(&self, chain_id: u64, block: BlockSelector) -> Result<Pob, RpcError> {
        let txs = self.trace_prestate(block)?;
        let mut unique = BTreeMap::new();
        let mut codes = BTreeMap::new();
        for tx in txs {
            if let Some(result) = tx.result {
                for (addr, acc) in result {
                    let code_hash = SH256::from(keccak_hash(&acc.code));
                    codes.entry(code_hash).or_insert(acc.code);
                    let acc_stateset = unique.entry(addr).or_insert_with(|| BTreeSet::new());
                    for key in acc.storage.keys() {
                        acc_stateset.insert(*key);
                    }
                }
            }
        }

        let mut blk = self.get_block(block)?;
        // blk.header.miner = "0x8F81e2E3F8b46467523463835F965fFE476E1c9E".into();
        // unique.entry(blk.header.miner.clone()).or_default();

        let mut fetch_reqs = Vec::with_capacity(unique.len());
        for (key, acc) in unique {
            fetch_reqs.push(FetchState {
                access_list: Some(Cow::Owned(TransactionAccessTuple {
                    address: key,
                    storage_keys: acc.into_iter().collect(),
                })),
                code: None,
            });
        }

        let prev_block = (blk.header.number.as_u64() - 1).into();

        let states = self.fetch_states(&fetch_reqs, prev_block, true)?;
        let prev_state_root = if blk.header.number.as_u64() > 0 {
            self.get_block_header(prev_block)?.state_root
        } else {
            SH256::default()
        };
        let block_hashes = BTreeMap::new();
        let pob = Pob::from_proof(chain_id, blk, prev_state_root, block_hashes, codes, states);
        Ok(pob)
    }

    pub fn trace_prestate(&self, block: BlockSelector) -> Result<Vec<TxPrestateResult>, RpcError> {
        let cli = self.client.raw();
        let cfg = TraceConfig {
            tracer: Some("prestateTracer".into()),
        };
        match block {
            BlockSelector::Hash(hash) => cli.rpc("debug_traceBlockByHash", (hash, cfg)),
            BlockSelector::Number(number) => cli.rpc("debug_traceBlockByNumber", (number, cfg)),
            BlockSelector::Latest => unimplemented!(),
        }
    }
}
