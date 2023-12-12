use std::prelude::v1::*;

use core::ops::DerefMut;
use crypto::keccak_hash;
use eth_types::{HexBytes, SH160, SH256, SU256, StateAccountTrait};
use std::borrow::Cow;
use std::collections::btree_map::Entry;
use std::time::Instant;
use std::{collections::BTreeMap, sync::Arc};

use super::{Error, NodeDB, StateDB, Trie, TrieCache, TrieCacheCtx};

#[derive(Debug)]
pub struct TrieState<T, N, A>
where
    T: Trie,
    N: NodeDB,
    A: StateAccountTrait,
{
    accounts: TrieCache<T, SH160, TrieStateAccount<A>>,
    storages: BTreeMap<SH160, Box<TrieCache<T, SH256, StorageValue>>>,
    db: N,
}

impl<T, N, A> TrieState<T, N, A>
where
    T: Trie<DB = N>,
    N: NodeDB,
    A: StateAccountTrait,
{
    pub fn new(trie: T, db: N) -> Self {
        Self {
            accounts: TrieCache::new(trie),
            storages: BTreeMap::new(),
            db,
        }
    }

    pub fn get_storage(&mut self, acc: &SH160) -> Option<&mut TrieCache<T, SH256, StorageValue>> {
        self.storages.get_mut(acc).map(|n| n.as_mut())
    }

    fn try_flush(&mut self) -> Vec<SH256> {
        let mut reduction_nodes = Vec::new();
        let start = Instant::now();
        let mut storage_dirty = 0;
        let mut account_dirty = 0;
        for (addr, item) in &mut self.accounts.cache {
            let mut dirty = self.accounts.dirty.contains_key(addr);
            // we already set the account dirty when the storage dirty
            if let Some(storage) = self.storages.get_mut(addr) {
                // don't try to assert the root matches because the state will changed if it has reduction nodes.
                // assert_eq!(storage.root_hash(), item.root.raw());
                storage_dirty += storage.dirty.len();
                if let Err(nodes) = storage.flush(&mut self.db) {
                    if nodes.len() > 0 {
                        reduction_nodes.extend(nodes);
                        continue;
                    }
                }
                item.update_root(&mut dirty, storage.root_hash().clone().into());
                glog::debug!("acc: {:?} => {:?}", addr, item);
            } else {
                // if we can't get the storage map, means we got no storage updates.
            }
            if dirty {
                self.accounts.dirty.insert(addr.clone(), ());
            }
        }
        let start_flush_acc = Instant::now();
        account_dirty += self.accounts.dirty.len();
        if let Err(nodes) = self.accounts.flush(&mut self.db) {
            reduction_nodes.extend(nodes);
        }
        glog::debug!(
            target: "trie_state",
            "flush storages({}): {:?}, flush acc({}): {:?}, total: {:?}, reduction: {}",
            storage_dirty,
            start_flush_acc - start,
            account_dirty,
            start_flush_acc.elapsed(),
            start.elapsed(),
            reduction_nodes.len(),
        );
        reduction_nodes.into_iter().map(|n| n.into()).collect()
    }
}

impl<T, N, A> Clone for TrieState<T, N, A>
where
    T: Trie + Clone,
    N: NodeDB + Clone,
    A: StateAccountTrait,
{
    fn clone(&self) -> Self {
        Self {
            accounts: self.accounts.clone(),
            storages: self.storages.clone(),
            db: self.db.clone(),
        }
    }
}

impl<T, N, A> StateDB for TrieState<T, N, A>
where
    T: Trie<DB = N> + Send + 'static,
    N: NodeDB + Send + 'static + Sync,
    A: StateAccountTrait,
{
    type StateAccount = A;
    fn add_balance(&mut self, address: &SH160, val: &SU256) -> Result<(), Error> {
        self.with_acc(address, |ctx| {
            glog::debug!(target: "statedb", "acc {:?} add balance {:?} = {:?}", address, val, ctx.val.balance()+val);
            ctx.val.set_balance(ctx.dirty, ctx.val.balance() + val);
        })
    }

    fn exist(&mut self, address: &SH160) -> Result<bool, Error> {
        self.with_acc(address, |ctx| ctx.val.is_exist())
    }

    fn flush(&mut self) -> Result<SH256, Error> {
        let reduction_nodes = self.try_flush();
        if reduction_nodes.len() > 0 {
            return Err(Error::Flush(format!(
                "should not have reduction nodes: {:?}",
                reduction_nodes
            )));
        }
        let hash = self.accounts.root_hash().clone();
        self.db.commit();
        Ok(hash)
    }

    fn fork(&self) -> Self {
        unimplemented!()
    }

    fn get_account_basic(&mut self, address: &SH160) -> Result<(SU256, u64), Error> {
        self.with_acc(address, |ctx| (ctx.val.balance(), ctx.val.nonce()))
    }

    fn get_balance(&mut self, address: &SH160) -> Result<SU256, Error> {
        self.with_acc(address, |ctx| ctx.val.balance())
    }

    fn get_code(&mut self, address: &SH160) -> Result<Arc<HexBytes>, Error> {
        if address.is_zero() {
            return Ok(Arc::new(HexBytes::new()));
        }
        self.with_acc(address, |ctx| {
            match ctx.db.get_code(&ctx.val.code_hash()) {
                Some(n) => Ok(n),
                None => return Err(Error::CodeNotFound(ctx.val.code_hash())),
            }
        })?
    }

    fn get_nonce(&mut self, address: &SH160) -> Result<u64, Error> {
        self.with_acc(address, |ctx| ctx.val.nonce())
    }

    fn get_state(&mut self, address: &SH160, index: &SH256) -> Result<SH256, Error> {
        let state = self.with_storage(address, index, |ctx| ctx.val.0);
        // glog::info!("get state {:?} -> {:?} = {:?}", address, index, state);
        state
    }

    fn revert(&mut self, root: SH256) {
        if self.accounts.revert(root.into()) {
            self.storages.clear();
            glog::info!("revert to: {:?}", root,);
        } else {
            glog::info!("not revert to: {:?}, no changed", root)
        }
    }

    fn set_balance(&mut self, address: &SH160, val: SU256) -> Result<(), Error> {
        glog::debug!(target: "statedb", "acc {:?} set balance to {:?}", address, val);
        self.with_acc(address, |ctx| ctx.val.set_balance(ctx.dirty, val))
    }

    fn set_code(&mut self, address: &SH160, code: Vec<u8>) -> Result<(), Error> {
        glog::debug!(target: "statedb", "acc {:?} set code: {:?}", address, keccak_hash(&code));
        self.with_acc(address, |ctx| ctx.val.set_code(ctx.dirty, code, ctx.db))
    }

    fn set_nonce(&mut self, address: &SH160, val: SU256) -> Result<(), Error> {
        glog::debug!(target: "statedb", "acc {:?} set nonce: {:?}", address, val);
        self.with_acc(address, |ctx| ctx.val.set_nonce(ctx.dirty, val.as_u64()))
    }

    fn set_state(&mut self, address: &SH160, index: &SH256, value: SH256) -> Result<(), Error> {
        self.with_storage(address, index, |ctx| {
            if ctx.val.0 == value {
                return;
            }
            glog::debug!(target: "statedb", "acc {:?} set state {:?}: {:?}", address, index, value);
            *ctx.dirty = true;
            ctx.val.0 = value;
        })
    }

    fn state_root(&self) -> SH256 {
        self.accounts.root_hash()
    }

    fn sub_balance(&mut self, address: &SH160, val: &SU256) -> Result<(), Error> {
        self.with_acc(address, |ctx| {
            glog::debug!(target: "statedb", "acc {:?} sub balance {:?} to {:?}", address, val, ctx.val.balance() - val);
            ctx.val.set_balance(ctx.dirty, ctx.val.balance() - val)
        })
    }

    fn suicide(&mut self, address: &SH160) -> Result<(), Error> {
        glog::debug!(target: "statedb", "acc {:?} suicide", address);
        self.with_acc(address, |ctx| ctx.val.suicide(ctx.dirty))
    }

    fn try_get_acc(&mut self, address: &SH160) -> Result<Option<A>, Error> {
        self.try_with_acc(address, |ctx| ctx.val.0.clone())
    }

    fn try_get_nonce(&mut self, address: &SH160) -> Option<u64> {
        self.try_with_acc(address, |ctx| ctx.val.nonce()).unwrap()
    }
}

impl<T, N, A> TrieState<T, N, A>
where
    T: Trie<DB = N> + Send + 'static,
    N: NodeDB + Send + 'static,
    A: StateAccountTrait,
{
    fn with_acc<Fn, O>(&mut self, address: &SH160, f: Fn) -> Result<O, Error>
    where
        Fn: FnOnce(TrieCacheCtx<'_, T, TrieStateAccount<A>, N>) -> O,
    {
        self.accounts
            .with_key(&mut self.db, address, f)
            .map_err(|err| Error::WithKey(err))
    }

    fn try_with_acc<Fn, O>(&mut self, address: &SH160, f: Fn) -> Result<Option<O>, Error>
    where
        Fn: FnOnce(TrieCacheCtx<'_, T, TrieStateAccount<A>, N>) -> O,
    {
        self.accounts
            .try_with_key(&mut self.db, address, f)
            .map_err(|err| Error::WithKey(err))
    }

    fn with_storage<Fn, O>(&mut self, address: &SH160, index: &SH256, f: Fn) -> Result<O, Error>
    where
        Fn: FnOnce(TrieCacheCtx<'_, T, StorageValue, N>) -> O,
    {
        let root = self.with_acc(address, |ctx| ctx.val.root())?;
        let storage = match self.storages.entry(address.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(n) => {
                let new_trie = self.accounts.raw().new_root(root.into());
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

#[derive(Debug, Default, Clone)]
pub struct TrieStateAccount<T: StateAccountTrait>(pub T);

impl<T: StateAccountTrait> rlp::Encodable for TrieStateAccount<T> {
    fn rlp_append(&self, s: &mut rlp::RlpStream) {
        if self.0.is_exist() {
            self.0.rlp_append(s)
        }
    }
}

impl<T: StateAccountTrait> rlp::Decodable for TrieStateAccount<T> {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        Ok(Self(rlp::Decodable::decode(rlp)?))
    }
}

impl<T: StateAccountTrait> std::ops::Deref for TrieStateAccount<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: StateAccountTrait> DerefMut for TrieStateAccount<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: StateAccountTrait> TrieStateAccount<T> {
    pub fn suicide(&mut self, dirty: &mut bool) {
        if self.0.is_exist() {
            self.0 = T::default();
            *dirty = true;
        }
    }

    pub fn set_balance(&mut self, dirty: &mut bool, val: SU256) {
        if self.0.set_balance(val) {
            *dirty = true;
        }
    }

    pub fn set_nonce(&mut self, dirty: &mut bool, val: u64) {
        if self.0.set_nonce(val) {
            *dirty = true;
        }
    }

    pub fn set_code<N: NodeDB>(&mut self, dirty: &mut bool, code: Vec<u8>, db: &mut N) {
        if self.0.set_code(&code) {
            let hash = keccak_hash(&code).into();
            db.set_code(hash, Cow::Owned(code.into()));
            *dirty = true;
        }
    }

    pub fn update_root(&mut self, dirty: &mut bool, root: SH256) {
        if self.0.update_root(root) {
            *dirty = true;
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct StorageValue(SH256);

impl std::ops::Deref for StorageValue {
    type Target = SH256;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl rlp::Encodable for StorageValue {
    fn rlp_append(&self, s: &mut rlp::RlpStream) {
        let idx = self.0.as_bytes().iter().position(|n| *n != 0);
        if idx.is_some() {
            // non-zero
            s.append_raw(&self.0 .0, 32);
        }
    }
}

impl rlp::Decodable for StorageValue {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        if rlp.is_null() {
            return Ok(StorageValue::default());
        }
        let raw = rlp.as_raw();
        assert_eq!(raw.len(), 32);
        let mut val = SH256::default();
        val.0.copy_from_slice(raw);
        Ok(Self(val))
    }
}
