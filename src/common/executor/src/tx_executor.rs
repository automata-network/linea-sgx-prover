use core::cmp::Ordering;
use std::{prelude::v1::*, time::Instant};

use base::format::parse_ether;
use eth_types::{Log, H160, H256, SU256};
use evm::{
    backend::Apply,
    executor::stack::{MemoryStackState, StackExecutor, StackSubstateMetadata},
    ExitFatal, ExitReason,
};
use statedb::StateDB;
use std::collections::BTreeMap;

use crate::{Context, ExecuteError, ExecuteResult, StateProxy};

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

    pub fn execute(&mut self) -> Result<ExecuteResult, ExecuteError> {
        let mut base_fee = self.ctx.header.base_fee_per_gas;

        self.check_nonce(false)?;
        self.check_base_fee(&mut base_fee)?;
        self.buy_gas()?;

        let result = self.exec_tx();
        if self.gas < result.used_gas && self.ctx.gas_overcommit {
            self.gas = 0;
        } else {
            self.gas -= result.used_gas;
        }
        let txfee = self.calculate_txfee(result.used_gas, &base_fee);
        self.apply_states(&result);

        if let Some(miner) = &self.ctx.miner {
            self.state_db
                .add_balance(miner, &txfee)
                .map_err(ExecuteError::StateError)?;
        }
        self.refund_gas()?;

        Ok(result)
    }

    fn calculate_txfee(&self, gas: u64, base_fee: &SU256) -> SU256 {
        let tx = &self.ctx.tx.tx;
        let gas_tip_cap = tx.max_priority_fee_per_gas();
        let gas_fee_cap = tx.max_fee_per_gas();
        let effective_tip = (*gas_tip_cap).min(*gas_fee_cap - base_fee);
        let extra_fee = self.ctx.extra_fee.unwrap_or(SU256::zero());

        SU256::from(gas) * &effective_tip + extra_fee
    }

    fn exec_tx(&mut self) -> ExecuteResult {
        let tx = &self.ctx.tx.tx;
        let precompile_set = self.ctx.precompile;
        let config = self.ctx.cfg;

        let access_list = self.generate_access_list();
        let gas_limit = self.ctx.tx.tx.gas().as_u64();

        let metadata = StackSubstateMetadata::new(gas_limit, config);
        let state = StateProxy::new(self.state_db, self.ctx.clone());

        let execute_instant = Instant::now();
        // glog::info!("gas remain: {}", metadata.gasometer().gas());
        let mem_state = MemoryStackState::new(metadata, &state);
        let mut executor = StackExecutor::new_with_precompiles(mem_state, config, precompile_set);

        // check balance > gas_limit * gasPrice first
        let (reason, data) = match tx.to() {
            Some(to) => executor.transact_call(
                self.ctx.caller.clone().into(),
                to.into(),
                tx.value().into(),
                tx.input().into(),
                gas_limit,
                access_list,
            ),
            None => executor.transact_create(
                self.ctx.caller.clone().into(),
                tx.value().into(),
                tx.input().into(),
                gas_limit,
                access_list,
            ),
        };

        let mut result = ExecuteResult {
            success: reason.is_succeed(),
            err: data.into(),
            used_gas: executor.used_gas(),
            logs: Vec::new(),
            states: Vec::new(),
        };

        if self.ctx.no_gas_fee {
            // executor.used_gas() will minus the refunded_gas but we don't need this feature when cost_gas_fee is disabled.
            use evm::executor::stack::StackState;
            let refund_gas = executor.state().metadata().gasometer().refunded_gas();
            result.used_gas += refund_gas as u64;
        }

        let (storages, logs) = executor.into_state().deconstruct();
        {
            let mut log_index = 0;
            for log in logs {
                result.logs.push(Log {
                    address: log.address.into(),
                    topics: log.topics.iter().map(|t| t.clone().into()).collect(),
                    data: log.data.clone().into(),
                    block_number: Default::default(),
                    transaction_hash: Default::default(),
                    transaction_index: Default::default(),
                    block_hash: Default::default(),
                    log_index: log_index.clone().into(),
                    removed: false,
                });
                log_index += 1;
            }
        }
        result.states = storages;

        result
    }

    fn check_base_fee(&self, base_fee: &mut SU256) -> Result<(), ExecuteError> {
        let gas_fee_cap = self.ctx.tx.tx.max_fee_per_gas();
        if gas_fee_cap < base_fee {
            let effective_gas_tip = self.ctx.tx.tx.effective_gas_tip(None).unwrap();
            return Err(ExecuteError::InsufficientBaseFee {
                tx_hash: self.ctx.tx.hash,
                block_base_fee_gwei: parse_ether(base_fee, 9),
                base_fee_gwei: parse_ether(&effective_gas_tip, 9),
                block_number: self.ctx.header.number.as_u64(),
            });
        }
        Ok(())
    }

    fn generate_access_list(&self) -> Vec<(H160, Vec<H256>)> {
        let tx = &self.ctx.tx.tx;
        let mut access_list = vec![];
        if let Some(al) = tx.access_list() {
            access_list.reserve(al.len());
            for tat in al {
                access_list.push((
                    tat.address.raw().clone(),
                    tat.storage_keys.iter().map(|n| n.raw().clone()).collect(),
                ));
            }
        }
        access_list
    }

    // check whether the caller's nonce matches the tx
    fn check_nonce(&mut self, try_get: bool) -> Result<(), ExecuteError> {
        let caller = self.ctx.caller;
        let tx_nonce = self.ctx.tx.tx.nonce();
        let nonce = if try_get {
            match self.state_db.try_get_nonce(caller) {
                Some(nonce) => nonce,
                None => return Ok(()),
            }
        } else {
            glog::debug!(target:"invalid_nonce", "check tx[{:?} {:?}] nonce", self.ctx.tx.hash, caller);
            self.state_db
                .get_nonce(caller)
                .map_err(ExecuteError::StateError)?
        };
        match nonce.cmp(&tx_nonce) {
            Ordering::Equal => Ok(()),
            Ordering::Greater => {
                return Err(ExecuteError::NonceTooLow {
                    got: tx_nonce,
                    expect: nonce,
                })
            }
            Ordering::Less => {
                return Err(ExecuteError::NonceTooHigh {
                    got: tx_nonce,
                    expect: nonce,
                });
            }
        }
    }

    fn buy_gas(&mut self) -> Result<(), ExecuteError> {
        let tx = &self.ctx.tx.tx;
        let caller = self.ctx.caller;
        let gas: SU256 = tx.gas().as_u64().into();
        let mut mgval = gas * self.gas_price;
        let mut balance_check = gas * tx.max_fee_per_gas();
        balance_check = balance_check + tx.value();
        let extra_fee = self.ctx.extra_fee.unwrap_or(SU256::default());
        balance_check += extra_fee;

        let balance = self
            .state_db
            .get_balance(caller)
            .map_err(ExecuteError::StateError)?;

        if balance < balance_check {
            // if !dry_run {
            glog::info!(
                "[{:?}] acc: {:?}, got balance: {}, need balance: {}",
                tx.hash().raw(),
                self.ctx.caller,
                balance,
                balance_check
            );
            return Err(ExecuteError::InsufficientFunds);
            // }

            // so the dry run can continue
            // mgval = balance;
        }

        self.gas += tx.gas().as_u64();

        self.initial_gas += tx.gas().as_u64();
        if !self.ctx.no_gas_fee {
            self.state_db
                .sub_balance(caller, &(extra_fee + mgval))
                .map_err(ExecuteError::StateError)?;
        }
        Ok(())
    }

    fn refund_gas(&mut self) -> Result<(), ExecuteError> {
        if !self.ctx.no_gas_fee {
            let remaining = SU256::from(self.gas) * self.gas_price;
            self.state_db
                .add_balance(self.ctx.caller, &remaining)
                .map_err(ExecuteError::StateError)?;
        }
        // glog::info!("refund gas fee: {}", remaining);
        Ok(())
    }

    fn apply_states(&mut self, result: &ExecuteResult) -> Result<(), ExecuteError> {
        for change in &result.states {
            match change {
                Apply::Modify {
                    address,
                    basic,
                    code,
                    storage,
                    reset_storage,
                } => {
                    let address = (*address).into();
                    if result.success {
                        if *reset_storage {
                            self.state_db
                                .suicide(&address)
                                .map_err(ExecuteError::StateError)?;
                        }

                        self.state_db
                            .set_balance(&address, basic.balance.into())
                            .map_err(ExecuteError::StateError)?;
                        self.state_db
                            .set_nonce(&address, basic.nonce.into())
                            .map_err(ExecuteError::StateError)?;
                        if let Some(code) = code {
                            self.state_db
                                .set_code(&address, code.clone())
                                .map_err(ExecuteError::StateError)?;
                        }
                        for (index, value) in storage {
                            self.state_db
                                .set_state(&address, &(*index).into(), (*value).into())
                                .map_err(ExecuteError::StateError)?;
                        }
                    } else {
                        if self.ctx.caller == &address {
                            self.state_db
                                .set_nonce(&address, basic.nonce.into())
                                .map_err(ExecuteError::StateError)?;
                        }
                    }
                }
                Apply::Delete { address } => {
                    if result.success {
                        self.state_db
                            .suicide(&(*address).into())
                            .map_err(ExecuteError::StateError)?;
                    } else {
                        // nothing to do
                    }
                }
            }
        }
        Ok(())
    }
}
