use std::prelude::v1::*;

use base::format::debug;
use core::fmt::Debug;
use eth_types::{FetchStateResult, HexBytes, StateAccount, SH160, SH256, SU256};
use mpt::StorageValue;
use statedb::{Error, MissingState};
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::Arc;
use zktrie::{Database, Node, Trace};

use crate::{Trie, TrieCache, TrieCacheCtx, ZkStateAccount, ZkTrie};

pub fn account_key(acc: &[u8]) -> SH256 {
    let hash = zktrie::hash(acc);
    hash
}

#[derive(Debug)]
pub struct ZkTrieState<D: Database<Node = Node>> {
    db: D,
    acc_cache: TrieCache<ZkTrie<D>, SH160, ZkStateAccount>,
    storages: BTreeMap<SH160, Box<TrieCache<ZkTrie<D>, SH256, StorageValue>>>,
    // root: ZkTrie<D>,
}

impl<D: Database<Node = Node>> ZkTrieState<D> {
    pub fn new_from_trace(db: D, t: &Trace) -> Self {
        let (next_free_node, sub_root) = t.old_state();
        let acc_cache = TrieCache::new(ZkTrie::new(next_free_node, sub_root));
        let storages = BTreeMap::new();
        ZkTrieState {
            db,
            acc_cache,
            storages,
        }
    }

    fn with_acc<Fn, O>(&mut self, address: &SH160, f: Fn) -> Result<O, Error>
    where
        Fn: FnOnce(TrieCacheCtx<'_, ZkTrie<D>, ZkStateAccount, D>) -> O,
    {
        self.acc_cache
            .with_key(&mut self.db, address, f)
            .map_err(|err| Error::WithKey(err))
    }

    fn with_storage<Fn, O>(&mut self, address: &SH160, index: &SH256, f: Fn) -> Result<O, Error>
    where
        Fn: FnOnce(TrieCacheCtx<'_, ZkTrie<D>, StorageValue, D>) -> O,
    {
        let root = self.with_acc(address, |ctx| ctx.val.root)?;
        let storage = match self.storages.entry(address.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(n) => {
                let new_trie = self
                    .acc_cache
                    .raw()
                    .new_root(&self.db, root.into())
                    .map_err(Error::WithKey)?;
                n.insert(Box::new(TrieCache::new(new_trie)))
            }
        };
        if storage.root_hash() != root {
            storage.revert(root);
        }
        let out = storage
            .with_key(&mut self.db, index, f)
            .map_err(|err| Error::WithKey(err))?;
        if storage.is_dirty(index) {
            self.with_acc(address, |ctx| *ctx.dirty = true)?;
        }
        Ok(out)
    }
}

impl<D: Database<Node = Node>> statedb::StateDB for ZkTrieState<D> {
    type StateAccount = StateAccount;
    fn add_balance(&mut self, address: &SH160, val: &SU256) -> Result<(), Error> {
        Ok(())
    }

    fn apply_states(&mut self, result: Vec<FetchStateResult>) -> Result<(), Error> {
        unreachable!()
    }

    fn check_missing_state(
        &mut self,
        address: &SH160,
        storages: &[SH256],
    ) -> Result<MissingState, Error> {
        unreachable!()
    }

    fn exist(&mut self, address: &SH160) -> Result<bool, Error> {
        unreachable!()
    }

    fn flush(&mut self) -> Result<SH256, Error> {
        unreachable!()
    }

    fn fork(&self) -> Self {
        unreachable!()
    }

    fn get_account_basic(&mut self, address: &SH160) -> Result<(SU256, u64), Error> {
        self.with_acc(address, |ctx| (ctx.val.balance, ctx.val.nonce))
    }

    fn get_balance(&mut self, address: &SH160) -> Result<SU256, Error> {
        self.with_acc(address, |ctx| ctx.val.balance)
    }

    fn get_code(&mut self, address: &SH160) -> Result<Arc<HexBytes>, Error> {
        let (code_hash, code_size) =
            self.with_acc(address, |ctx| (ctx.val.keccak_code_hash, ctx.val.code_size))?;
        if code_size == 0 {
            return Ok(Arc::new(HexBytes::new()));
        }
        match self.db.get_code(&code_hash) {
            Some(code) => Ok(code),
            None => Err(Error::CodeNotFound(code_hash)),
        }
    }

    fn get_nonce(&mut self, address: &SH160) -> Result<u64, Error> {
        self.with_acc(address, |ctx| ctx.val.nonce)
    }

    fn get_state(&mut self, address: &SH160, index: &SH256) -> Result<SH256, Error> {
        glog::info!("get stat: {:?} {:?}", address, index);
        self.with_storage(address, index, |ctx| {
            glog::info!("ctx: {:?}", ctx.val);
        });
        unreachable!()
    }

    fn revert(&mut self, root: SH256) {
        unreachable!()
    }

    fn set_balance(&mut self, address: &SH160, val: SU256) -> Result<(), Error> {
        unreachable!()
    }

    fn set_code(&mut self, address: &SH160, code: Vec<u8>) -> Result<(), Error> {
        unreachable!()
    }

    fn set_nonce(&mut self, address: &SH160, val: SU256) -> Result<(), Error> {
        unreachable!()
    }

    fn set_state(&mut self, address: &SH160, index: &SH256, value: SH256) -> Result<(), Error> {
        unreachable!()
    }

    fn state_root(&self) -> SH256 {
        unreachable!()
    }

    fn sub_balance(&mut self, address: &SH160, val: &SU256) -> Result<(), Error> {
        self.with_acc(address, |ctx| {
            *ctx.dirty = true;
            ctx.val.balance -= val;
        })
    }

    fn suicide(&mut self, address: &SH160) -> Result<(), Error> {
        unreachable!()
    }

    fn try_get_acc(&mut self, address: &SH160) -> Result<Option<Self::StateAccount>, Error> {
        unreachable!()
    }

    fn try_get_nonce(&mut self, address: &SH160) -> Option<u64> {
        unreachable!()
    }
}
