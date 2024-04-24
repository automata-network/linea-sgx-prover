use core::marker::PhantomData;
use std::prelude::v1::*;

use eth_types::{HexBytes, SH256};
use std::sync::Arc;

use crate::{
    format_node_index, init_world_state, trie_hash, utils, Database, Error, FlattenedLeaf,
    LeafOpening, LeafType, MemStore, Node, SpareMerkleTrie, ZK_TRIE_DEPTH,
};

pub type MemZkTrie = ZkTrie<MemStore>;

#[derive(Debug)]
pub struct ZkTrie<D: Database<Node = Node> + std::fmt::Debug> {
    state: SpareMerkleTrie,
    _marker: PhantomData<D>,
}

impl<D: Database<Node = Node> + std::fmt::Debug> ZkTrie<D> {
    pub fn new(db: &D, root: SH256) -> Result<Self, Error> {
        Ok(ZkTrie {
            state: SpareMerkleTrie::new(db, root)?,
            _marker: PhantomData,
        })
    }

    pub fn new_from_sub_root(next_free_node: u64, sub_root: SH256) -> Self {
        ZkTrie {
            state: SpareMerkleTrie::new_from_sub_root(next_free_node, sub_root),
            _marker: PhantomData,
        }
    }

    pub fn empty(db: &mut D) -> Result<Self, Error> {
        let (empty_node, _) = init_world_state();
        let mut trie = Self::new(db, *empty_node.hash())?;
        trie.set_head_and_tail(db)?;
        Ok(trie)
    }

    fn set_head_and_tail(&mut self, db: &mut D) -> Result<(), Error> {
        let index = self.get_next_free_leaf_node_index(&db)?;
        let head_path = utils::get_leaf_path(index);
        self.state
            .put(db, &head_path, LeafOpening::head().to_bytes())?;
        db.update_index(LeafOpening::head().hkey, FlattenedLeaf::head().clone());

        self.increment_next_free_leaf_node_index(db)?;
        let tail_index = self.get_next_free_leaf_node_index(&db)?;
        db.update_index(LeafOpening::tail().hkey, FlattenedLeaf::tail().clone());

        let tail_path = utils::get_leaf_path(tail_index);
        self.state
            .put(db, &tail_path, LeafOpening::tail().to_bytes())?;
        self.increment_next_free_leaf_node_index(db)?;
        Ok(())
    }

    pub fn top_root_hash(&self) -> &SH256 {
        self.state.root_hash()
    }

    pub fn sub_root_hash(&self) -> &SH256 {
        self.state.sub_root_hash()
    }

    pub fn next_free_node(&self) -> Option<u64> {
        self.state.next_free_node()
    }

    fn get_next_free_node_path(&self) -> [u8; 2] {
        const NEXT_FREE_NODE_PATH: u8 = 0;
        [NEXT_FREE_NODE_PATH, LeafType::NextFreeNode as u8]
    }

    pub fn increment_next_free_leaf_node_index(&mut self, db: &mut D) -> Result<(), Error> {
        let found_free_node = self
            .state
            .next_free_node()
            .ok_or(Error::InMemNextNodeNotFound)?;
        let next_free_node = found_free_node + 1;
        self.state.set_next_free_node(next_free_node);
        Ok(())
    }

    fn get_next_free_leaf_node_index(&self, db: &D) -> Result<u64, Error> {
        // todo: cache?
        let path = &self.get_next_free_node_path();
        let node = self.state.get_node(db, path)?;
        let index = node
            .map(|node| {
                node.value()
                    .map(utils::parse_node_index)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        Ok(index)
    }

    fn parse_node<N, F: FnOnce(&[u8]) -> N>(&self, db: &D, key: &[u8], f: F) -> Result<N, Error> {
        match self.state.get_node(db, key) {
            Ok(Some(node)) => match node.value() {
                Some(data) => Ok(f(data)),
                None => Err(Error::ZkTrieParseNodeFail(key.into(), node.raw().ty())),
            },
            Ok(None) => Err(Error::ZKTrieKeyNotFound(key.into())),
            Err(err) => Err(err),
        }
    }

    pub fn commit(&mut self, db: &mut D) {
        self.state.commit(db);
    }

    pub fn remove(&mut self, db: &mut D, key: &[u8]) -> Result<(), Error> {
        let hkey = trie_hash(key)?;
        let nearest_key = db.get_nearest_keys(self.top_root_hash(), &hkey);
        match &nearest_key.center {
            Some(current_flat_leaf_value) => {
                let left_leaf_path = nearest_key.left_path();
                let right_leaf_path = nearest_key.right_path();

                // UPDATE HKey- with HKey+ for next
                {
                    let prior_left_leaf =
                        self.parse_node(db, &left_leaf_path, LeafOpening::parse)?;
                    let new_left_leaf =
                        prior_left_leaf.new_next_leaf(nearest_key.right_index.into());
                    self.state
                        .put(db, &left_leaf_path, new_left_leaf.to_bytes())?;
                }

                // REMOVE hash(k)
                {
                    let leaf_path_to_delete =
                        utils::get_leaf_path(current_flat_leaf_value.leaf_index);
                    let prior_deleted_leaf =
                        self.parse_node(db, &leaf_path_to_delete, LeafOpening::parse)?;
                    db.remove_index(&hkey);

                    self.state.remove(db, &leaf_path_to_delete)?;
                }

                // UPDATE HKey+ with HKey- for prev
                {
                    let prior_right_leaf =
                        self.parse_node(db, &right_leaf_path, LeafOpening::parse)?;
                    let new_right_leaf =
                        prior_right_leaf.new_prev_leaf(nearest_key.left_index.into());
                    self.state
                        .put(db, &right_leaf_path, new_right_leaf.to_bytes())?;
                }
            }
            None => (),
        }
        Ok(())
    }

    pub fn read(&self, db: &D, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let hkey = trie_hash(key)?;
        let nearest_keys = db.get_nearest_keys(self.top_root_hash(), &hkey);
        Ok(match nearest_keys.center {
            Some(leaf) => Some(leaf.leaf_value.into()),
            None => None,
        })
    }

    pub fn put(&mut self, db: &mut D, key: &[u8], value: Vec<u8>) -> Result<(), Error> {
        let hkey = trie_hash(key)?;
        let nearest_keys = db.get_nearest_keys(self.top_root_hash(), &hkey);
        match nearest_keys.center {
            None => {
                let left_leaf_path = nearest_keys.left_path();
                let right_leaf_path = nearest_keys.right_path();
                let next_free_node = self
                    .state
                    .next_free_node()
                    .ok_or(Error::InMemNextNodeNotFound)?;

                // UPDATE HKey- with hash(k) for next
                {
                    let prior_left_leaf =
                        self.parse_node(db, &left_leaf_path, LeafOpening::parse)?;
                    let new_left_leaf = prior_left_leaf.new_next_leaf(next_free_node.into());

                    self.state
                        .put(db, &left_leaf_path, new_left_leaf.to_bytes())?;

                    let leaf_path_to_add = utils::get_leaf_path(next_free_node);
                    let hval = trie_hash(&value)?;
                    db.update_index(hkey, FlattenedLeaf::new(next_free_node, value.into()));

                    let new_leaf_value = LeafOpening::new(
                        nearest_keys.left_index,
                        nearest_keys.right_index,
                        hkey,
                        hval,
                    );

                    self.state
                        .put(db, &leaf_path_to_add, new_leaf_value.to_bytes())?;
                }

                // UPDATE HKey+ with hash(k) for prev
                {
                    let prior_right_leaf =
                        self.parse_node(db, &right_leaf_path, LeafOpening::parse)?;
                    let new_right_leaf = prior_right_leaf.new_prev_leaf(next_free_node.into());
                    self.state
                        .put(db, &right_leaf_path, new_right_leaf.to_bytes())?;
                }

                self.increment_next_free_leaf_node_index(db)?;
            }
            Some(current_flat_leaf_value) => {
                let leaf_path_to_update = current_flat_leaf_value.leaf_path();

                let hval = trie_hash(&value)?;
                db.update_index(
                    hkey,
                    FlattenedLeaf::new(current_flat_leaf_value.leaf_index, value.into()),
                );

                let prior_updated_leaf =
                    self.parse_node(db, &leaf_path_to_update, LeafOpening::parse)?;

                let new_updated_leaf = prior_updated_leaf.new_hval(hval);

                self.state
                    .put(db, &leaf_path_to_update, new_updated_leaf.to_bytes())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::time::Instant;

    use base::format::debug;

    use crate::{
        init_world_state, trie_hash, utils, LeafOpening, MemStore, Node, NodeValue, Trace,
        ZK_TRIE_DEPTH,
    };

    use super::*;

    #[test]
    fn test_worldstate_head_and_tail() {
        let val = LeafOpening::head().to_bytes();
        let hash = trie_hash(&val).unwrap();
        assert_eq!(
            hash,
            "0x0891fa77c3d0c9b745840d71d41dcb58b638d4734bb4f0bba4a3d1a2d847b672".into()
        );
        let val = LeafOpening::tail().to_bytes();
        let hash = trie_hash(&val).unwrap();
        assert_eq!(
            hash,
            "0x10ba2286f648a549b50ea5f1b6e1155d22c31eb4727c241e76c420200cd5dbe0".into()
        );
    }

    #[test]
    fn test_empty_worldstate() {
        let (default_node, _) = init_world_state();
        assert_eq!(
            *default_node.hash(),
            "0x09349798db316b1b222f291207e9e1368e9b887a234dcc73b433e6218a43f173".into()
        );
    }

    #[test]
    fn test_zktrie_empty_roothash() {
        let mut db = MemStore::new();
        let mut empty_trie = MemZkTrie::empty(&mut db).unwrap();
        assert_eq!(
            empty_trie.top_root_hash(),
            &"0x07977874126658098c066972282d4c85f230520af3847e297fe7524f976873e5".into()
        );
        assert_eq!(
            empty_trie.sub_root_hash(),
            &"0x0951bfcd4ac808d195af8247140b906a4379b3f2d37ec66e34d2f4a5d35fa166".into()
        );
    }

    #[test]
    fn test_zktrie_insertion_root_hash() {
        glog::init_test();
        let mut db = MemStore::new();
        let mut trie = MemZkTrie::empty(&mut db).unwrap();
        let key = utils::create_dum_digest(58).0.to_vec();
        let value = utils::create_dum_digest(42).0.to_vec();

        trie.put(&mut db, &key, value).unwrap();
        assert_eq!(
            trie.sub_root_hash(),
            &"0x0882afe875656680dceb7b17fcba7c136cec0c32becbe9039546c79f71c56d36".into()
        );
        assert_eq!(
            trie.top_root_hash(),
            &"0x0cfdc3990045390093be4e1cc9907b220324cccd1c8ea9ede980c7afa898ef8d".into()
        );
    }

    #[test]
    fn test_zktrie_insertion_and_update_root_hash() {
        glog::init_test();
        let mut db = MemStore::new();
        let mut trie = MemZkTrie::empty(&mut db).unwrap();
        let key = utils::create_dum_digest(58).0.to_vec();
        let dum_value = utils::create_dum_digest(41).0.to_vec();
        let new_dum_value = utils::create_dum_digest(42).0.to_vec();

        trie.put(&mut db, &key, dum_value).unwrap();
        assert_eq!(
            trie.sub_root_hash(),
            &"0x02703cefa95c6dd143543c5e73b14e51a3b714dc73816c6830e4267a41792b1a".into()
        );
        assert_eq!(
            trie.top_root_hash(),
            &"0x03b9554192a170e9424f8cdcd5657ce1826123d93239b9aeb24a648d67522aa5".into()
        );

        trie.put(&mut db, &key, new_dum_value).unwrap();

        assert_eq!(
            trie.sub_root_hash(),
            &"0x0882afe875656680dceb7b17fcba7c136cec0c32becbe9039546c79f71c56d36".into()
        );
        assert_eq!(
            trie.top_root_hash(),
            &"0x0cfdc3990045390093be4e1cc9907b220324cccd1c8ea9ede980c7afa898ef8d".into()
        );
    }

    #[test]
    fn test_zktrie_insertion_and_delete_root_hash() {
        let mut db = MemStore::new();
        let mut trie = MemZkTrie::empty(&mut db).unwrap();
        let key = utils::create_dum_digest(58).0.to_vec();
        let value = utils::create_dum_digest(41).0.to_vec();
        trie.put(&mut db, &key, value).unwrap();

        assert_eq!(
            trie.sub_root_hash(),
            &"0x02703cefa95c6dd143543c5e73b14e51a3b714dc73816c6830e4267a41792b1a".into()
        );
        assert_eq!(
            trie.top_root_hash(),
            &"0x03b9554192a170e9424f8cdcd5657ce1826123d93239b9aeb24a648d67522aa5".into()
        );

        trie.remove(&mut db, &key).unwrap();
        assert_eq!(
            trie.sub_root_hash(),
            &"0x0951bfcd4ac808d195af8247140b906a4379b3f2d37ec66e34d2f4a5d35fa166".into()
        );
        assert_eq!(
            trie.top_root_hash(),
            &"0x0bcb88342825fa7a079a5cf5f77d07b1590a140c311a35acd765080eea120329".into()
        );
    }

    fn get_traces(path: &str) -> Result<Vec<Trace>, String> {
        let data = std::fs::read_to_string(format!("testdata/{}.hex", path)).map_err(debug)?;
        let mut traces = Vec::new();
        for line in data.split("\n") {
            let line = HexBytes::from_hex(line.as_bytes()).map_err(debug)?;
            if line.len() == 0 {
                continue;
            }
            let trace = rlp::decode(&line).map_err(debug)?;
            traces.push(trace);
        }
        Ok(traces)
    }

    #[test]
    pub fn test_from_proof() {
        let prev_state_root: SH256 =
            "0x108e0450f48e7b3a9420bc085a9da6704e1da76ac4898eef9db5afe4ff48b2ab".into();
        let next_free_node = 66;

        let traces = get_traces("from_proof").unwrap();
        let mut db = MemStore::from_traces(traces.clone()).unwrap();
        let mut trie = MemZkTrie::new_from_sub_root(next_free_node, prev_state_root);

        assert_eq!(
            trie.top_root_hash(),
            &"0x0e321f8eb2495968e6367ea5c4c7617c91040f6cbcf8a2fba6096bcfded21f2a".into()
        );

        let key = utils::create_dum_digest(9).0.to_vec();
        assert_eq!(trie.read(&db, &key), Ok(None));

        {
            // trace[1]: READ 10
            let key = utils::create_dum_digest(10).0.to_vec();
            let result = trie.read(&db, &key).unwrap();
            assert_eq!(result, Some(utils::create_dum_digest(10).0.to_vec()));
        }

        {
            // trace[2]: UPDATE 12 => 120
            let key = utils::create_dum_digest(12).0.to_vec();
            let value = utils::create_dum_digest(120).0.to_vec();
            trie.put(&mut db, &key, value).unwrap();
            assert_eq!(
                trie.top_root_hash(),
                &"0x0540718e2b8049263ac9f7284c333a72cf9b7b0db99ee0f90d0985224d3f7789".into()
            );
        }
        {
            // trace[3]: INSERT 11 => 120
            let key = utils::create_dum_digest(11).0.to_vec();
            let value = utils::create_dum_digest(120).0.to_vec();
            trie.put(&mut db, &key, value).unwrap();
            assert_eq!(
                trie.top_root_hash(),
                &"0x0d01a39a97f703890fdf899ba1f8f6ce44dde04cabc32559bc79ed311e93a4a0".into()
            );
        }
        {
            // trace[4]: DELETE 14
            let key = utils::create_dum_digest(14).0.to_vec();
            trie.remove(&mut db, &key).unwrap();
            assert_eq!(
                trie.top_root_hash(),
                &"0x035adf5e7ff80f4e60bf4f1d2d499343fcd0010e4fc7a1c9ca9281f01834f075".into()
            );
        }
        {
            // trace[5]: READ ZERO 14
            let key = utils::create_dum_digest(14).0.to_vec();
            let result = trie.read(&mut db, &key).unwrap();
            assert_eq!(
                trie.top_root_hash(),
                &"0x035adf5e7ff80f4e60bf4f1d2d499343fcd0010e4fc7a1c9ca9281f01834f075".into()
            );
            assert_eq!(result, None);
        }
    }

    #[test]
    fn performance() {
        let traces = get_traces("performance").unwrap();
        println!("{}", traces.len());
        let mut db = MemStore::from_traces(traces.clone()).unwrap();
        let next_free_node = 642;
        let prev_state_root =
            "0x09bb97d5c671eb0224bab1a44d8d60b35f5226253db52faddde77dcb228fd455".into();
        let mut trie = MemZkTrie::new_from_sub_root(next_free_node, prev_state_root);
        println!("{:?}", trie.top_root_hash());

        let now = Instant::now();
        for i in (1..1281).step_by(2) {
            let key = utils::create_dum_digest(i).0;
            let value = utils::create_dum_digest(i).0.to_vec();
            trie.put(&mut db, &key, value).unwrap()
        }
        assert_eq!(trie.top_root_hash(), &"0x0bc22636292bf4e78e57136ed7b945fed0279aba402e911a6a326935dbba19c6".into());
        println!("insert {:?}", now.elapsed());

        let now = Instant::now();
        for i in 1..1281 {
            let key = utils::create_dum_digest(i).0;
            trie.read(&mut db, &key).unwrap();
        }
        assert_eq!(trie.top_root_hash(), &"0x0bc22636292bf4e78e57136ed7b945fed0279aba402e911a6a326935dbba19c6".into());
        println!("read {:?}", now.elapsed());

        let now = Instant::now();
        for i in (1..1281).step_by(2) {
            let key = utils::create_dum_digest(i).0;
            let value = utils::create_dum_digest(i*2).0.to_vec();
            trie.put(&mut db, &key, value).unwrap();
        }
        assert_eq!(trie.top_root_hash(), &"0x051a87b46bf3a6f9389eb6b04c663e9974457b8d3dfc89c8d780c53b195c747f".into());
        println!("update {:?}", now.elapsed());

        let now = Instant::now();
        for i in (0..1280).step_by(2) {
            let key = utils::create_dum_digest(i).0;
            trie.remove(&mut db, &key).unwrap();
        }
        assert_eq!(trie.top_root_hash(), &"0x08d09cb60beab4f503896ce7c8bbe20717d7369db730967870d3079b9068a435".into());
        println!("remove {:?}", now.elapsed());

        let now = Instant::now();
        for i in (0..1280).step_by(2) {
            let key = utils::create_dum_digest(i).0;
            trie.read(&mut db, &key).unwrap();
        }
        assert_eq!(trie.top_root_hash(), &"0x08d09cb60beab4f503896ce7c8bbe20717d7369db730967870d3079b9068a435".into());
        println!("read zero {:?}", now.elapsed());
    }
}
