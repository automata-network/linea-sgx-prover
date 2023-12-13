use std::prelude::v1::*;

use crate::{add_nodes, Trie, TrieNode, TrieStorageNode, TrieUpdateResult};
use base::trace::AvgCounterResult;
use crypto::keccak_hash;
use eth_types::{
    BlockHeader, FetchState, FetchStateResult, HexBytes, StateAccount, SH160, SH256, SU256,
};
use statedb::{Error, NodeDB, StateDB, TrieStateAccount};

use std::borrow::Cow;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Instant;

pub type AccountMap = TrieMap<SH160, TrieStateAccount<StateAccount>>;
pub type StorageMap = TrieMap<SH256, StorageValue>;

#[derive(Debug)]
pub struct TrieState<F, D: NodeDB> {
    accounts: AccountMap,
    storages: BTreeMap<SH160, Box<StorageMap>>,
    db: D,
    fetcher: F,
}

pub type NoStateFetcher = ();

impl ProofFetcher for NoStateFetcher {
    fn fetch_proofs(&self, key: &[u8]) -> Result<Vec<HexBytes>, String> {
        Err(format!("key not found for proofs: {:?}", key))
    }

    fn get_nodes(&self, node: &[SH256]) -> Result<Vec<HexBytes>, String> {
        Err(format!("nodes not found: {:?}", node))
    }
}

impl StateFetcher for NoStateFetcher {
    fn fork(&self) -> Self {
        ()
    }

    fn get_account(&self, address: &SH160) -> Result<(SU256, u64, HexBytes), Error> {
        Err(Error::WithKey(format!("account[{:?}] not found", address)))
    }

    fn get_block_hash(&self, number: u64) -> Result<SH256, Error> {
        Err(Error::WithKey(format!(
            "block_hash[{:?}] not found",
            number
        )))
    }

    fn get_code(&self, address: &SH160) -> Result<HexBytes, Error> {
        Err(Error::WithKey(format!(
            "account code[{:?}] not found",
            address
        )))
    }

    fn get_miss_usage(&self) -> AvgCounterResult {
        AvgCounterResult::default()
    }

    fn get_storage(&self, address: &SH160, key: &SH256) -> Result<SH256, Error> {
        Err(Error::WithKey(format!(
            "account storage[{:?} {:?}] not found",
            address, key
        )))
    }

    fn prefetch_states(
        &self,
        list: &[FetchState],
        with_proof: bool,
    ) -> Result<Vec<FetchStateResult>, Error> {
        unimplemented!()
    }

    fn with_acc(&self, address: &SH160) -> Self {
        ()
    }
}

pub trait StateFetcher: Debug + Send + 'static + ProofFetcher + Clone {
    fn with_acc(&self, address: &SH160) -> Self;
    fn get_block_hash(&self, number: u64) -> Result<SH256, Error>;
    fn get_code(&self, address: &SH160) -> Result<HexBytes, Error>;
    fn get_account(&self, address: &SH160) -> Result<(SU256, u64, HexBytes), Error>;
    fn get_storage(&self, address: &SH160, key: &SH256) -> Result<SH256, Error>;
    fn fork(&self) -> Self;
    fn get_miss_usage(&self) -> AvgCounterResult;
    fn prefetch_states(
        &self,
        list: &[FetchState],
        with_proof: bool,
    ) -> Result<Vec<FetchStateResult>, Error>;
}

impl<F, S> Clone for TrieState<F, S>
where
    F: StateFetcher,
    S: NodeDB<Node = TrieStorageNode> + Send + 'static,
{
    fn clone(&self) -> Self {
        self.fork()
    }
}

impl<F, S> TrieState<F, S>
where
    F: StateFetcher,
    S: NodeDB<Node = TrieStorageNode> + Send + 'static,
{
    pub fn new(fetcher: F, state_root: SH256, db: S) -> Self {
        Self {
            fetcher,
            accounts: AccountMap::new(state_root.into()),
            storages: BTreeMap::new(),
            db,
        }
    }

    pub fn account_trie(&self) -> &TrieNode {
        self.accounts.root()
    }

    pub fn storege_trie(&self, account: &SH160) -> Option<&TrieNode> {
        self.storages.get(account).map(|n| n.root())
    }

    fn with_acc<Fn, O>(&mut self, address: &SH160, f: Fn) -> Result<O, Error>
    where
        Fn: FnOnce(TrieMapCtx<'_, TrieStateAccount<StateAccount>, S, F>) -> O,
    {
        self.accounts
            .with_key(&mut self.db, &self.fetcher, address, f)
            .map_err(|err| Error::DecodeError(err))
    }

    fn with_storage<Fn, O>(&mut self, address: &SH160, index: &SH256, f: Fn) -> Result<O, Error>
    where
        Fn: FnOnce(TrieMapCtx<'_, StorageValue, S, F>) -> O,
    {
        let root = self.with_acc(address, |ctx| ctx.val.root)?;
        let storage = match self.storages.entry(address.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(n) => n.insert(Box::new(TrieMap::new(root.into()))),
        };
        if storage.root_hash() != &root {
            storage.revert(root);
        }
        let storage_fetcher = self.fetcher.with_acc(address);
        let out = storage
            .with_key(&mut self.db, &storage_fetcher, index, f)
            .map_err(|err| Error::DecodeError(err))?;
        if storage.dirty.contains_key(index) {
            self.with_acc(address, |ctx| *ctx.dirty = true)?;
        }
        Ok(out)
    }

    fn try_with_acc<Fn, O>(&mut self, address: &SH160, f: Fn) -> Result<Option<O>, Error>
    where
        Fn: FnOnce(TrieMapCtx<'_, TrieStateAccount<StateAccount>, S, ()>) -> O,
    {
        self.accounts
            .try_with_key(&mut self.db, address, f)
            .map_err(|err| Error::DecodeError(err))
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
                let nodes = storage.flush(&mut self.db);
                if nodes.len() > 0 {
                    reduction_nodes.extend(nodes);
                    continue;
                }
                item.update_root(&mut dirty, storage.root_hash().clone().into());
            } else {
                // if we can't get the storage map, means we got no storage updates.
            }
            if dirty {
                self.accounts.dirty.insert(addr.clone(), ());
            }
        }
        let start_flush_acc = Instant::now();
        account_dirty += self.accounts.dirty.len();
        let nodes = self.accounts.flush(&mut self.db);
        reduction_nodes.extend(nodes);
        glog::info!(
            exclude:"dry_run",
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

impl<F, S> StateDB for TrieState<F, S>
where
    F: StateFetcher,
    S: NodeDB<Node = TrieStorageNode> + Send + 'static,
{
    type StateAccount = StateAccount;
    fn state_root(&self) -> SH256 {
        self.accounts.root_hash().clone().into()
    }

    fn try_get_acc(&mut self, address: &SH160) -> Result<Option<StateAccount>, Error> {
        self.try_with_acc(address, |ctx| ctx.val.0.clone())
    }

    fn add_balance(&mut self, address: &SH160, val: &SU256) -> Result<(), Error> {
        glog::debug!(target: "state_change", "account[{:?}]: add balance: {:?}", address, val);
        self.with_acc(address, |ctx| {
            ctx.val.set_balance(ctx.dirty, ctx.val.balance + val);
        })
    }

    fn set_balance(&mut self, address: &SH160, val: SU256) -> Result<(), Error> {
        glog::debug!(target: "state_change", "account[{:?}]: set balance: {:?}", address, val);
        self.with_acc(address, |ctx| ctx.val.set_balance(ctx.dirty, val))
    }

    fn sub_balance(&mut self, address: &SH160, val: &SU256) -> Result<(), Error> {
        self.with_acc(address, |ctx| {
            glog::debug!(target: "state_change", "account[{:?}]: sub balance: {} - {} -> {:?}", address, ctx.val.balance, val, ctx.val.balance-val);
            ctx.val.set_balance(ctx.dirty, ctx.val.balance - val)
        })
    }

    fn set_nonce(&mut self, address: &SH160, val: SU256) -> Result<(), Error> {
        glog::debug!(target: "state_change", "account[{:?}]: set nonce: {:?}", address, val);
        self.with_acc(address, |ctx| ctx.val.set_nonce(ctx.dirty, val.as_u64()))
    }

    fn get_nonce(&mut self, address: &SH160) -> Result<u64, Error> {
        // glog::debug!(target: "state_get", "account[{:?}]: get nonce: {:?}", address, val);
        self.with_acc(address, |ctx| ctx.val.nonce)
    }

    fn try_get_nonce(&mut self, address: &SH160) -> Option<u64> {
        self.try_with_acc(address, |ctx| ctx.val.nonce).unwrap()
    }

    fn set_code(&mut self, address: &SH160, code: Vec<u8>) -> Result<(), Error> {
        glog::debug!(target: "state_change", "account[{:?}]: set code", address);
        self.with_acc(address, |ctx| ctx.val.set_code(ctx.dirty, code, ctx.store))
    }

    fn get_code(&mut self, address: &SH160) -> Result<Arc<HexBytes>, Error> {
        let fetcher = self.fetcher.fork();
        self.with_acc(address, |ctx| {
            glog::debug!(target: "state_get", "account[{:?}]: get code, hash:{:?}", address, HexBytes::from(ctx.val.code_hash.as_bytes()));
            match ctx.store.get_code(&ctx.val.code_hash) {
                Some(data) => Ok(data.into()),
                None => {
                    let code = fetcher.get_code(address)?;
                    let hash = keccak_hash(&code).into();
                    ctx.store.set_code(hash, Cow::Borrowed(&code));
                    Ok(Arc::new(code))
                },
            }
        })?
    }

    fn set_state(&mut self, address: &SH160, index: &SH256, value: SH256) -> Result<(), Error> {
        glog::debug!(target: "state_change", "account[{:?}]: set_state, {:?} -> {:?}", address, index.raw(), value.raw());
        self.with_storage(address, index, |ctx| {
            if ctx.val.0 == value {
                return;
            }
            *ctx.dirty = true;
            ctx.val.0 = value;
        })
    }

    fn get_account_basic(&mut self, address: &SH160) -> Result<(SU256, u64), Error> {
        self.with_acc(address, |ctx| (ctx.val.balance, ctx.val.nonce))
    }

    fn revert(&mut self, root: SH256) {
        if self.accounts.revert(root.into()) {
            self.storages.clear();
            glog::info!(
                "revert to: {:?}, accounts: {:?}, storages: {:?}",
                root,
                self.accounts,
                self.storages
            );
        } else {
            glog::info!("not revert to: {:?}, no changed", root)
        }
    }

    fn flush(&mut self) -> Result<SH256, Error> {
        let mut reduction_nodes = self.try_flush();
        if reduction_nodes.len() > 0 {
            let nodes = self
                .fetcher
                .get_nodes(&reduction_nodes)
                .map_err(|err| Error::CallRemoteFail(format!("{:?}", err)))?;
            let nodes = TrieNode::from_proofs(&self.db, &nodes).unwrap();
            add_nodes(&mut self.db, nodes);
            reduction_nodes = self.try_flush();
            assert_eq!(reduction_nodes.len(), 0);
        }

        self.db.commit();
        let hash = self.accounts.root_hash().clone().into();
        assert!(
            self.db.get(&hash).is_some(),
            "fail to load root: {:?}",
            hash
        );
        // glog::info!("trieState: after flush: {:?}", hash);
        Ok(hash)
    }

    fn get_balance(&mut self, address: &SH160) -> Result<SU256, Error> {
        self.with_acc(address, |ctx| ctx.val.balance)
    }

    fn exist(&mut self, address: &SH160) -> Result<bool, Error> {
        self.with_acc(address, |ctx| ctx.val.is_exist())
    }

    fn suicide(&mut self, address: &SH160) -> Result<(), Error> {
        glog::debug!(target: "state_change", "account[{:?}]: suicide", address);
        self.with_acc(address, |ctx| ctx.val.suicide(ctx.dirty))?;
        self.storages.remove(address);
        Ok(())
    }

    fn fork(&self) -> Self {
        Self {
            fetcher: self.fetcher.clone(),
            accounts: AccountMap::new(self.state_root().into()),
            storages: BTreeMap::new(),
            db: self.db.fork(),
        }
    }

    fn get_state(&mut self, address: &SH160, index: &SH256) -> Result<SH256, Error> {
        let val = self.with_storage(address, index, |ctx| ctx.val.0.into())?;
        // glog::info!("get state: {:?}.{:?} => {:?}", address, index, val);
        Ok(val)
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
        if let Some(idx) = idx {
            s.append(&&self.0.as_bytes()[idx..]);
        }
    }
}

impl rlp::Decodable for StorageValue {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        if rlp.is_null() {
            return Ok(StorageValue::default());
        }
        let data = rlp.data()?;
        assert!(data.len() <= 32);
        let mut out = SH256::default();
        out.0[32 - data.len()..].copy_from_slice(&data);
        Ok(Self(out))
    }
}

impl<K, V> From<Trie> for TrieMap<K, V> {
    fn from(trie: Trie) -> Self {
        Self {
            trie,
            cache: BTreeMap::new(),
            dirty: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrieMap<K, V> {
    trie: Trie,
    cache: BTreeMap<K, V>,
    dirty: BTreeMap<K, ()>,
}

pub trait ProofFetcher {
    fn fetch_proofs(&self, key: &[u8]) -> Result<Vec<HexBytes>, String>;
    fn get_nodes(&self, node: &[SH256]) -> Result<Vec<HexBytes>, String>;
    // fn get_node(&self, key: &HexBytes) -> Result<HexBytes, String>;
}

pub struct TrieMapCtx<'a, V, S, F> {
    pub val: &'a mut V,
    pub dirty: &'a mut bool,
    pub store: &'a mut S,
    pub fetcher: &'a F,
}

impl<'a, V, S, F> TrieMapCtx<'a, V, S, F> {
    pub fn new(store: &'a mut S, val: &'a mut V, dirty: &'a mut bool, fetcher: &'a F) -> Self {
        Self {
            val,
            store,
            dirty,
            fetcher,
        }
    }
}

impl<K, V> TrieMap<K, V>
where
    K: AsRef<[u8]> + Clone + Debug + Ord,
    V: rlp::Decodable + rlp::Encodable + Default + Debug + Clone,
{
    pub fn new(root: TrieNode) -> Self {
        let trie = Trie::new(root);
        let cache = BTreeMap::new();
        let dirty = BTreeMap::new();
        Self { trie, cache, dirty }
    }

    pub fn root(&self) -> &TrieNode {
        self.trie.root()
    }

    pub fn root_hash(&self) -> &SH256 {
        self.trie.root().hash()
    }

    pub fn revert(&mut self, root: SH256) -> bool {
        if self.trie.root().hash() == &root {
            if self.dirty.len() == 0 {
                return false;
            }
        }
        self.cache.clear();
        self.dirty.clear();
        self.trie = Trie::new(root.into());
        return true;
    }

    pub fn must_flush<S, F>(&mut self, store: &mut S, fetcher: &F) -> Result<(), String>
    where
        F: ProofFetcher,
        S: NodeDB<Node = TrieStorageNode>,
    {
        let reduction_nodes = self.flush(store);
        if reduction_nodes.len() > 0 {
            let reduction_nodes: Vec<SH256> =
                reduction_nodes.into_iter().map(|n| n.into()).collect();
            let nodes = fetcher.get_nodes(&reduction_nodes)?;
            let nodes = TrieNode::from_proofs(store, &nodes).map_err(|err| format!("{:?}", err))?;
            add_nodes(store, nodes);
            let reduction_nodes = self.flush(store);
            assert_eq!(reduction_nodes.len(), 0);
        }
        Ok(())
    }

    pub fn flush<S>(&mut self, store: &mut S) -> Vec<SH256>
    where
        S: NodeDB<Node = TrieStorageNode>,
    {
        let mut trie = self.trie.clone();
        let mut reduction_nodes = Vec::new();
        let mut removed = Vec::with_capacity(self.dirty.len());
        let mut updated = 0;
        for (k, _) in &mut self.dirty {
            if let Some(v) = self.cache.get(k) {
                let data: Vec<u8> = rlp::encode(v).into();
                let data_hex = HexBytes::from(data.as_slice());
                let update_result = trie.update(store, k.as_ref(), data);
                glog::debug!(target:"flush_state", "[flush-state] set {:?} => {:?}, rlp: {}", k, v, data_hex);

                match update_result {
                    TrieUpdateResult::NewTrie(new_trie) => {
                        updated += 1;
                        trie = new_trie
                    }
                    TrieUpdateResult::NoChanged => {}
                    TrieUpdateResult::MissingNode(hash) => {
                        unreachable!("should not missing node: {:?}", hash)
                    }
                    TrieUpdateResult::ReductionNode(hash) => {
                        reduction_nodes.push(hash);
                        continue;
                    }
                }
            }
            removed.push(k.clone());
        }
        for key in &removed {
            self.dirty.remove(key);
        }

        if self.trie.root().hash() != trie.root().hash() {
            self.trie = trie;
        }

        let root = self.trie.root();
        if root.embedded().is_some() {
            assert!(
                store.get(&root.hash()).is_some(),
                "TrieMap flush fail: root not found: {:?}, updated = {}, trie: {:?}",
                root,
                updated,
                self.trie.root(),
            );
        }
        reduction_nodes
    }

    pub fn try_with_key<S, FN, O>(
        &mut self,
        store: &mut S,
        k: &K,
        f: FN,
    ) -> Result<Option<O>, rlp::DecoderError>
    where
        S: NodeDB<Node = TrieStorageNode>,
        FN: FnOnce(TrieMapCtx<'_, V, S, ()>) -> O,
    {
        if let Some(v) = self.cache.get_mut(k) {
            let mut dirty = false;
            let ctx = TrieMapCtx::new(store, v, &mut dirty, &());
            let out = f(ctx);
            if dirty {
                self.dirty.insert(k.clone(), ());
            }
            return Ok(Some(out));
        }
        let origin_key = k.as_ref();
        // let mut fetched = false;
        let result = self.trie.get(store, origin_key);
        match result.get_data() {
            Ok(data) => {
                let data: V = if data.len() == 0 {
                    V::default()
                } else {
                    rlp::decode(data)?
                };
                let v = self.cache.entry(k.clone()).or_insert(data);
                let mut dirty = false;
                let ctx = TrieMapCtx::new(store, v, &mut dirty, &());
                let out = f(ctx);
                if dirty {
                    self.dirty.insert(k.clone(), ());
                }
                return Ok(Some(out));
            }
            Err(_) => {
                return Ok(None);
            }
        }
    }

    pub fn get_cloned<S, F>(
        &mut self,
        store: &mut S,
        fetcher: &F,
        k: &K,
    ) -> Result<V, rlp::DecoderError>
    where
        F: ProofFetcher + Debug,
        S: NodeDB<Node = TrieStorageNode>,
    {
        self.with_key(store, fetcher, k, |ctx| ctx.val.clone())
    }

    pub fn with_key<S, F, FN, O>(
        &mut self,
        store: &mut S,
        fetcher: &F,
        k: &K,
        f: FN,
    ) -> Result<O, rlp::DecoderError>
    where
        F: ProofFetcher + Debug,
        S: NodeDB<Node = TrieStorageNode>,
        FN: FnOnce(TrieMapCtx<'_, V, S, F>) -> O,
    {
        if let Some(v) = self.cache.get_mut(k) {
            let mut dirty = false;
            let ctx = TrieMapCtx::new(store, v, &mut dirty, fetcher);
            let out = f(ctx);
            if dirty {
                self.dirty.insert(k.clone(), ());
            }
            return Ok(out);
        }
        let origin_key = k.as_ref();
        let mut fetched = false;
        loop {
            let result = self.trie.get(store, origin_key);
            match result.get_data() {
                Ok(data) => {
                    let data: V = if data.len() == 0 {
                        V::default()
                    } else {
                        rlp::decode(data)?
                    };
                    let v = self.cache.entry(k.clone()).or_insert(data);
                    let mut dirty = false;
                    let ctx = TrieMapCtx::new(store, v, &mut dirty, fetcher);
                    let out = f(ctx);
                    if dirty {
                        self.dirty.insert(k.clone(), ());
                    }
                    return Ok(out);
                }
                Err(missing_hash) => {
                    glog::info!("missing_hash: {:?}", missing_hash);
                    if !fetched {
                        fetched = true;
                        let data = self.fetch_key(store, fetcher, origin_key).unwrap();
                        if !data.contains(&missing_hash) {
                            // println!("{}", self.root().format(store));
                            panic!(
                                "missing key: {}, hash: {:?}, nodes: {:?}",
                                HexBytes::from(origin_key),
                                missing_hash,
                                data,
                            );
                        }
                    } else {
                        unreachable!("should have hash")
                    }
                }
            }
        }
    }

    fn fetch_key<S, F>(
        &self,
        store: &mut S,
        fetcher: &F,
        origin_key: &[u8],
    ) -> Result<Vec<SH256>, String>
    where
        F: ProofFetcher + Debug,
        S: NodeDB<Node = TrieStorageNode>,
    {
        let proofs = fetcher.fetch_proofs(origin_key)?;
        let proof_nodes =
            TrieNode::from_proofs(store, &proofs).map_err(|err| format!("{:?}", err))?;
        for node in &proof_nodes {
            store.add_node(node.embedded().expect("should be embedded node"));
        }
        // if proof_nodes.len() == 0 {
        //     glog::info!(
        //         "got zero nodes: {:?}, proofs:{:?}, fetcher: {:?}",
        //         HexBytes::from(origin_key),
        //         proofs,
        //         fetcher
        //     );
        // }
        // sanity check?
        Ok(proof_nodes.iter().map(|n| n.hash().clone()).collect())
    }
}
