use std::prelude::v1::*;

use app::getargs::{Opt, Options};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub l2: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ServerConfig {
    pub tls: String,
    pub body_limit: usize,
    pub workers: usize,
}

#[derive(Debug)]
pub struct Args {
    pub executable: String,
    pub port: u32,
    pub cfg: String,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            executable: "".into(),
            port: 18400,
            cfg: "config/prover.json".into(),
        }
    }
}

impl Args {
    pub fn from_args(mut args: Vec<String>) -> Self {
        let mut out = Args::default();
        out.executable = args.remove(0);
        let mut opts = Options::new(args.iter().map(|a| a.as_str()));
        while let Some(opt) = opts.next_opt().expect("argument parsing error") {
            match opt {
                Opt::Short('p') => {
                    out.port = opts.value().unwrap().parse().unwrap();
                }
                Opt::Short('c') => {
                    out.cfg = opts.value().unwrap().parse().unwrap();
                }
                opt => {
                    glog::warn!("unknown opt: {:?}", opt);
                    continue;
                }
            }
        }
        out
    }
}
