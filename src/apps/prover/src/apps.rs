use core::time::Duration;
use std::prelude::v1::*;

use app::{Const, Getter, Var, VarMutex};
use base::{
    format::debug,
    fs::read_file,
    thread::{self, parallel},
    trace::Alive,
};
use crypto::{keccak_encode, Secp256r1PrivateKey, Secp256r1Signature};
use eth_tools::ExecutionClient;
use eth_types::{Block, EthereumEngineTypes, HexBytes, SH160, SH256, SU64};
use evm_executor::{BlockBuilder, Poe};
use jsonrpc::{JsonrpcErrorObj, MixRpcClient, RpcArgs, RpcServer, RpcServerConfig};
use linea::{
    account_key, BatchTask, BatchTaskSubscriber, BatchTaskSubscriberConfig, BlockExecutor, Prover,
    Verifier, ZkStateAccount, ZkTrieState,
};
use mpt::Database;
use shomei::RollupgetZkEVMStateMerkleProofV0Resp;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use zktrie::{PrefixDB, Trace};

use crate::{Args, Config};

#[derive(Default)]
pub struct App {
    pub alive: Alive,
    pub args: Const<Args>,
    pub server: VarMutex<RpcServer<Api>>,
    pub l2: Var<ExecutionClient<Arc<MixRpcClient>>>,
    pub rollup_el: Var<RollupExecutionClient>,
    pub verifier_el: Var<VerifierExecutionClient>,
    pub verifier: Var<Verifier<Arc<MixRpcClient>, EthereumEngineTypes>>,
    pub prover: Var<Prover>,
    pub cfg: Var<Config>,
    pub build_context: Var<BuildContext>,
    pub shomei: Var<shomei::Client>,
}

impl app::App for App {
    fn run(&self, env: app::AppEnv) -> Result<(), String> {
        self.args.set(Args::from_args(env.args));
        let cfg = self.cfg.get(self);
        let rollup_el = self.rollup_el.get(self);

        let verifier = self.verifier.get(self);
        let prover = self.prover.get(self);
        let l2 = self.l2.get(self);
        let build_ctx = self.build_context.get(self);

        // thread::spawn("prover-attested-monitor".into(), {
        //     let prover = prover.clone();
        //     let relay_account = cfg.verifier.relay_account.clone();
        //     let verifier = verifier.clone();
        //     move || {
        //         prover.monitor_attested(&relay_account, &verifier, |prvkey| {
        //             let prover_key = prvkey.public().eth_accountid().into();
        //             verifier.generate_prover_report(&prover_key, false)
        //         });
        //     }
        // });

        thread::spawn("jsonrpc".into(), {
            let srv = self.server.get(self);
            let alive = self.alive.clone();
            move || {
                let mut srv = srv.lock().unwrap();
                srv.run();
            }
        })
        .join()
        .unwrap();

        // let subscriber = {
        //     let cfg = BatchTaskSubscriberConfig {
        //         tag: format!("batch-task"),
        //         contract: cfg.rollup.contract,
        //         max_block: cfg.rollup.max_block,
        //         wait_block: cfg.rollup.wait_block,
        //     };
        //     BatchTaskSubscriber::new(self.alive.clone(), cfg, rollup_el.0.clone())
        // };
        // subscriber
        //     .subscribe({
        //         let alive = self.alive.clone();
        //         let prover = prover.clone();
        //         move |task| {
        //             if !prover.wait_attested(&alive) {
        //                 glog::error!("cancel task: {:?}", task);
        //                 return;
        //             }
        //             if let Err(err) = build_ctx.execute_task(task.clone()) {
        //                 glog::error!("prove task[{:?}] failed: {:?}", task.blocks, err);
        //                 return;
        //             };
        //         }
        //     })
        //     .unwrap();

        Ok(())
    }

    fn terminate(&self) {
        self.alive.shutdown()
    }
}

#[derive(Clone)]
pub struct BuildContext {
    pub alive: Alive,
    pub cfg: Arc<Config>,
    pub prover: Arc<Prover>,
    pub verifier: Arc<Verifier<Arc<MixRpcClient>, EthereumEngineTypes>>,
    pub l2: Arc<ExecutionClient<Arc<MixRpcClient>>>,
    pub shomei: Arc<shomei::Client>,
}

impl BuildContext {
    fn submit_poe(&self, task: BatchTask, poe: Poe) -> Result<(), String> {
        let prover = &self.prover;
        let verifier = &self.verifier;
        let cfg = &self.cfg;

        let batch_id = (*task.blocks.last().unwrap()).into();
        let report = poe.encode();

        verifier.commit_batch(&cfg.verifier.relay_account, &batch_id, &report)?;
        Ok(())
    }

    fn generate_poe_v2(
        &self,
        chain_id: u64,
        result: RollupgetZkEVMStateMerkleProofV0Resp,
        block: Block,
    ) -> Result<(), String> {
        glog::info!("start root_hash: {:?}", result.zk_parent_state_root_hash);
        glog::info!("end root_hash: {:?}", result.zk_end_state_root_hash);
        let block_trace = &result.zk_state_merkle_proof[0];
        let current_block = (block.header.number.as_u64() - 1).into();
        let proofs = self
            .shomei
            .fetch_proof_by_traces(&block_trace, current_block)
            .map_err(debug)?;

        for item in block_trace {
            glog::info!("traces: {:?}", item);
        }

        let mut codes = Vec::new();
        for t in block_trace {
            if t.location().len() == 0 {
                let value = t.read_value();
                if value.len() > 0 {
                    let acc = ZkStateAccount::from_bytes(value);
                    let mut addr = SH160::default();
                    addr.0.copy_from_slice(t.key());
                    codes.push(addr);
                }
            }
        }
        let codes = self.l2.get_codes(&codes, current_block).map_err(debug)?;

        let mut db = zktrie::MemStore::from_traces(&block_trace).map_err(debug)?;
        db.add_codes(codes);

        for proof in proofs {
            let hkey = account_key(&proof.account_proof.key);
            let root_hash = if let Some((leaf_index, proof)) = proof.account_proof.inclusion() {
                db.add_proof(
                    u64::max_value(),
                    leaf_index,
                    hkey,
                    proof.value.as_ref().map(|n| n.as_bytes()),
                    &proof.proof_related_nodes,
                )
                .map_err(debug)?
            } else {
                glog::info!("proof: {:?}", proof.account_proof);
                continue;
            };
            glog::info!(
                "[{:?}] state root_hash: {:?}",
                proof.account_proof.key,
                root_hash
            );
        }

        let be = BlockExecutor::new(chain_id.into());
        be.execute_v2(
            PrefixDB::new(u64::max_value(), db.into()),
            &block_trace,
            block,
        )?;
        Ok(())
    }

    fn generate_poe(&self, chain_id: u64, task: BatchTask) -> Result<Poe, String> {
        let l2 = &self.l2;
        let be = BlockExecutor::new(chain_id.into());
        let alive = self.alive.clone();

        let batch_result = Arc::new(Mutex::new(BTreeMap::new()));

        glog::info!("task: {:?}", task);
        parallel(&alive, task.blocks.clone(), 8, {
            let batch_result = batch_result.clone();
            let be = be.clone();
            let l2 = l2.clone();
            move |blk| {
                let handle = || -> Result<(Block, Poe), String> {
                    let mut pob = be.generate_pob(l2.as_ref(), blk.into())?;
                    let state_hash = pob.state_hash();
                    let prev_state_root = pob.data.prev_state_root;
                    let withdrawal_root = SH256::default();
                    let expect_state_root = pob.block.header.state_root;
                    let db = Database::new();
                    let block = be.execute(&db, pob)?;
                    let poe = Poe::single_block(
                        state_hash,
                        prev_state_root,
                        expect_state_root,
                        withdrawal_root,
                    );
                    Ok((block, poe))
                };
                let result = handle();
                let mut batch_result = batch_result.lock().unwrap();
                batch_result.insert(blk, result);
                Ok(())
            }
        });

        let batch_result = match Arc::try_unwrap(batch_result) {
            Ok(n) => n,
            Err(n) => {
                unreachable!("result should unwrap: {}", Arc::strong_count(&n))
            }
        };

        let batch_result = batch_result.into_inner().unwrap();
        if batch_result.len() != task.blocks.len() {
            // some task was panic
            return Err(format!(
                "some task failed, tasks: {}, finished: {}",
                task.blocks.len(),
                batch_result.len()
            ));
        }
        let mut poes = Vec::with_capacity(batch_result.len());
        for (_, result) in batch_result {
            let (block, poe) = result?;
            if poe.new_state_root != block.header.state_root {
                return Err(format!(
                    "block[{}] state root mismatch, remote: {:?}, local: {:?}",
                    block.header.number, poe.new_state_root, block.header.state_root
                ));
            }
            poes.push(poe);
        }

        // FIXME: data_hash should be calculated by the data instead of fetching from the chain
        let mut poe = Poe::batch(task.data_hash, &poes)?;
        let prover_key = self.prover.get_prvkey();
        poe.sign(&chain_id.into(), &prover_key);
        Ok(poe)
    }

    fn execute_task(&self, task: BatchTask) -> Result<(), String> {
        let chain_id = self.l2.chain_id().map_err(debug)?;
        let poe = self.generate_poe(chain_id, task.clone())?;
        self.submit_poe(task, poe)?;
        Ok(())
    }
}

pub struct Api {
    build_context: Arc<BuildContext>,
}

impl Api {
    fn test(&self, arg: RpcArgs<(u64,)>) -> Result<(), JsonrpcErrorObj> {
        let block_number = arg.params.0;
        let result = self
            .build_context
            .shomei
            .fetch_proof(block_number, block_number)
            .unwrap();

        let chain_id = self
            .build_context
            .l2
            .chain_id()
            .map_err(|e| JsonrpcErrorObj::server("fetch chain id fail", e))?;

        let block = self
            .build_context
            .l2
            .get_block(block_number.into())
            .map_err(JsonrpcErrorObj::unknown)?;
        glog::info!("blk: {:?}", block);

        self.build_context
            .generate_poe_v2(chain_id, result, block)
            .map_err(JsonrpcErrorObj::client)?;
        Ok(())
    }

    fn prove(&self, arg: RpcArgs<(SU64, SU64)>) -> Result<Poe, JsonrpcErrorObj> {
        let start = arg.params.0.as_u64();
        let end = arg.params.1.as_u64();
        if end < start {
            return Err(JsonrpcErrorObj::client(format!(
                "invalid block range: {}..{}",
                start, end
            )));
        }
        let chain_id = self
            .build_context
            .l2
            .chain_id()
            .map_err(|e| JsonrpcErrorObj::server("fetch chain id fail", e))?;

        let task = BatchTask {
            data_hash: SH256::default(),
            blocks: (start..=end).collect(),
        };
        let poe = self
            .build_context
            .generate_poe(chain_id, task)
            .map_err(JsonrpcErrorObj::client)?;
        Ok(poe)
    }
}

impl Getter<shomei::Client> for App {
    fn generate(&self) -> shomei::Client {
        let cfg = self.cfg.get(self);
        shomei::Client::new(&self.alive, cfg.shomei.clone())
    }
}

impl Getter<RpcServer<Api>> for App {
    fn generate(&self) -> RpcServer<Api> {
        let args = self.args.get();
        let cfg = self.cfg.get(self);
        let (tls_cert, tls_key) = match cfg.server.tls.as_str() {
            "" => (Vec::new(), Vec::new()),
            path => (
                read_file(&format!("{}.crt", path)).unwrap().into(),
                read_file(&format!("{}.key", path)).unwrap().into(),
            ),
        };

        let api = Api {
            build_context: self.build_context.get(self),
        };
        let server_cfg = RpcServerConfig {
            listen_addr: format!("0.0.0.0:{}", args.port),
            tls_cert,
            tls_key,
            http_max_body_length: Some(cfg.server.body_limit),
            ws_frame_size: 64 << 10,
            threads: cfg.server.workers,
            max_idle_secs: Some(60),
        };
        let mut srv = RpcServer::new(self.alive.clone(), server_cfg, Arc::new(api)).unwrap();
        srv.jsonrpc("prove", Api::prove);
        srv.jsonrpc("test", Api::test);
        srv
    }
}

impl Getter<Config> for App {
    fn generate(&self) -> Config {
        let data = base::fs::read_file(&self.args.get().cfg).unwrap();
        let cfg: Config = serde_json::from_slice(&data).unwrap();
        cfg
    }
}

pub struct RollupExecutionClient(pub ExecutionClient<Arc<MixRpcClient>>);
pub struct VerifierExecutionClient(pub ExecutionClient<Arc<MixRpcClient>>);

impl Getter<RollupExecutionClient> for App {
    fn generate(&self) -> RollupExecutionClient {
        let mut client = MixRpcClient::new(None);
        client
            .add_endpoint(&self.alive, &[self.cfg.get(self).rollup.endpoint.clone()])
            .unwrap();
        RollupExecutionClient(ExecutionClient::new(Arc::new(client)))
    }
}

impl Getter<VerifierExecutionClient> for App {
    fn generate(&self) -> VerifierExecutionClient {
        let mut client = MixRpcClient::new(None);
        client
            .add_endpoint(&self.alive, &[self.cfg.get(self).verifier.endpoint.clone()])
            .unwrap();
        VerifierExecutionClient(ExecutionClient::new(Arc::new(client)))
    }
}

impl Getter<ExecutionClient<Arc<MixRpcClient>>> for App {
    fn generate(&self) -> ExecutionClient<Arc<MixRpcClient>> {
        let mut client = MixRpcClient::new(None);
        client
            .add_endpoint(&self.alive, &[self.cfg.get(self).l2.clone()])
            .unwrap();
        ExecutionClient::new(Arc::new(client))
    }
}

impl Getter<Verifier<Arc<MixRpcClient>, EthereumEngineTypes>> for App {
    fn generate(&self) -> Verifier<Arc<MixRpcClient>, EthereumEngineTypes> {
        let el = &self.verifier_el.get(self).0;
        let chain_id = el.chain_id().unwrap();
        let resend_timeout = Duration::from_secs(45);
        let cfg = self.cfg.get(self);

        let verifier = Verifier::new(
            &self.alive,
            chain_id,
            el.clone(),
            resend_timeout,
            cfg.verifier.contract,
        );
        verifier
    }
}

impl Getter<Prover> for App {
    fn generate(&self) -> Prover {
        Prover::new(self.alive.clone())
    }
}

impl Getter<BuildContext> for App {
    fn generate(&self) -> BuildContext {
        let build_ctx = BuildContext {
            alive: self.alive.clone(),
            cfg: self.cfg.get(self),
            prover: self.prover.get(self),
            verifier: self.verifier.get(self),
            l2: self.l2.get(self),
            shomei: self.shomei.get(self),
        };
        build_ctx
    }
}
