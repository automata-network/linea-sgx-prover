use std::prelude::v1::*;

use eth_types::SU256;
use statedb::StateDB;

use crate::Context;

#[derive(Debug)]
pub struct TxExecutor<'a, D: StateDB> {
    ctx: Context<'a>,
    state_db: &'a mut D,
    initial_gas: u64,
    gas: u64,
    gas_price: SU256,
}

impl<'a, D: StateDB> TxExecutor<'a, D> {
    pub fn new(ctx: Context<'a>, state_db: &'a mut D) -> Self {
        let gas_price = ctx.tx.tx.gas_price(Some(ctx.header.base_fee_per_gas));
        Self {
            ctx,
            state_db,
            gas: 0,
            initial_gas: 0,
            gas_price,
        }
    }
}