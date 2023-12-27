use std::prelude::v1::*;

use app::{Const, Getter, Var, VarMutex};
use base::{
    format::debug,
    fs::read_file,
    trace::{Alive, AliveIter},
};
use eth_tools::ExecutionClient;
use evm_executor::{BlockBuilder, ConsensusBlockInfo, Engine, Ethereum};
use jsonrpc::{JsonrpcErrorObj, MixRpcClient, RpcArgs, RpcServer, RpcServerConfig};
use linea::{BlockExecutor, Linea};
use mpt::{Database, Trie};
use statedb::{MemStore, NodeDB};
use std::sync::Arc;

use crate::{Args, Config};

#[derive(Default)]
pub struct App {
    pub alive: Alive,
    pub args: Const<Args>,
    pub serve: VarMutex<RpcServer<Api>>,
    pub l2: Var<ExecutionClient<Arc<MixRpcClient>>>,
    pub cfg: Var<Config>,
}

pub struct Fetcher {}

impl app::App for App {
    fn run(&self, env: app::AppEnv) -> Result<(), String> {
        self.args.set(Args::from_args(env.args));
        let l2 = self.l2.get(self);
        let chain_id = l2.chain_id().map_err(debug)?;

        for i in self.alive.iter(2600106..2600107) {
            let be = BlockExecutor::new(chain_id.into());
            let pob = be.generate_pob(l2.as_ref(), i.into()).unwrap();
            let db = Database::new(100000);
            be.execute(&db, pob).unwrap();
        }
        glog::info!("All blocks executed successfully!");
        Ok(())
    }

    fn terminate(&self) {
        self.alive.shutdown()
    }
}

pub struct Api {}

impl Api {
    fn test(&self, arg: RpcArgs<()>) -> Result<(), JsonrpcErrorObj> {
        Ok(())
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

        let api = Api {};
        let server_cfg = RpcServerConfig {
            listen_addr: format!("0.0.0.0:{}", args.port),
            tls_cert,
            tls_key,
            http_max_body_length: Some(cfg.server.body_limit),
            ws_frame_size: 64 << 10,
            threads: cfg.server.workers,
        };
        let mut srv = RpcServer::new(self.alive.clone(), server_cfg, Arc::new(api)).unwrap();
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

impl Getter<ExecutionClient<Arc<MixRpcClient>>> for App {
    fn generate(&self) -> ExecutionClient<Arc<MixRpcClient>> {
        let mut client = MixRpcClient::new(None);
        client
            .add_endpoint(&self.alive, &[self.cfg.get(self).l2.clone()])
            .unwrap();
        ExecutionClient::new(Arc::new(client))
    }
}
