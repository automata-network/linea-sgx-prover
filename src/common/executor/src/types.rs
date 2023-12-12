use std::prelude::v1::*;

use eth_types::{PoolTx, BlockHeader, SU256, SH256, SH160};

use crate::PrecompileSet;

pub enum ExecuteError {
    NotSupported,
    InsufficientFunds,
    InsufficientBaseFee {
        tx_hash: SH256,
        block_base_fee_gwei: String,
        base_fee_gwei: String,
        block_number: u64,
    },
    ExecutePaymentTxFail(String),
    NonceTooLow {
        expect: u64,
        got: u64,
    },
    NonceTooHigh {
        expect: u64,
        got: u64,
    },
    StateError(statedb::Error),
}

#[derive(Debug, Clone)]
pub struct Context<'a> {
    pub chain_id: &'a SU256,
    pub caller: &'a SH160,
    pub cfg: &'a evm::Config,
    pub precompile: &'a PrecompileSet,
    pub tx: &'a PoolTx,
    pub header: &'a BlockHeader,
}
