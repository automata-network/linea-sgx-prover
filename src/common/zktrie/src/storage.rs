use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::{HexBytes, SH256};
use lazy_static::lazy_static;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    trie_hash, utils, Database, Error, LeafOpening, Node, NodeValue, Trace, EMPTY_TRIE_NODE,
    ZK_TRIE_DEPTH,
};

pub struct PrefixDB<'a> {
    prefix: u64,
    raw: RawDB<'a, MemStore>,
}

impl<'a> PrefixDB<'a> {
    pub fn new(prefix: u64, raw: RawDB<'a, MemStore>) -> Self {
        Self { prefix, raw }
    }
}

pub enum RawDB<'a, D> {
    Borrowed(&'a mut D),
    Owned(D),
}

impl<'a, D> From<D> for RawDB<'a, D> {
    fn from(db: D) -> Self {
        RawDB::Owned(db)
    }
}

impl<'a, D> From<&'a mut D> for RawDB<'a, D> {
    fn from(db: &'a mut D) -> Self {
        RawDB::Borrowed(db)
    }
}

impl<'a, D> std::ops::Deref for RawDB<'a, D> {
    type Target = D;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(n) => n,
            Self::Owned(n) => &n,
        }
    }
}

impl<'a, D> std::ops::DerefMut for RawDB<'a, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Borrowed(n) => n,
            Self::Owned(n) => n,
        }
    }
}

impl<'a> Database for PrefixDB<'a> {
    type Node = Node;
    fn get_code(&mut self, hash: &SH256) -> Option<Arc<HexBytes>> {
        self.raw.get_code(hash)
    }

    fn get_nearest_keys(&self, root: &SH256, k: &SH256) -> KeyRange {
        self.raw.get_nearest_keys(self.prefix, root, k)
    }

    fn get_node(&self, key: &SH256) -> Result<Option<Arc<Self::Node>>, Error> {
        self.raw.get_node(key)
    }

    fn remove_index(&mut self, k: &SH256) {
        self.raw.remove_index(k)
    }

    fn update_index(&mut self, k: SH256, v: FlattenedLeaf) {
        self.raw.update_index(k, v)
    }

    fn update_node(&mut self, key: SH256, node: Self::Node) -> Result<Arc<Self::Node>, Error> {
        self.raw.update_node(key, node)
    }
}

#[derive(Debug)]
pub struct MemStore {
    nodes: BTreeMap<SH256, Arc<Node>>,
    index: LevelMap,
    staging: BTreeMap<SH256, FlattenedLeaf>,
    codes: BTreeMap<SH256, Arc<HexBytes>>,
}

pub struct MemStoreSlot<'a> {
    prefix: u64,
    db: &'a mut MemStore,
}

#[derive(Debug)]
pub struct LevelMap {
    // root: SH256,
    // down: Option<Arc<LevelMap>>,
    vals: BTreeMap<SH256, BTreeMap<(u64, SH256), KeyRange>>,
}

impl LevelMap {
    pub fn from_traces(traces: &[Trace]) -> Result<Self, Error> {
        let mut idx = 0;
        let mut base = LevelMap::new();
        loop {
            let prefix = u64::max_value();
            let trace = &traces[idx];
            let top_hash = trace.old_top_hash();
            let root_map = base.vals.entry(top_hash).or_insert_with(|| BTreeMap::new());
            let hkey = trie_hash(trace.key())?;
            root_map.insert((prefix, hkey), trace.key_range());

            idx += 1;
            if idx >= traces.len() {
                break;
            }
        }
        for (item, key) in &base.vals {
            glog::info!("{}", "=".repeat(30));
            glog::info!("key: {:?} => {:?}", item, key.keys());
            for t in traces {
                if &t.old_top_hash() == item {
                    glog::info!("key={:?}, trace={:?}", trie_hash(t.key()).unwrap(), t);
                }
            }
        }
        Ok(base)
    }

    pub fn new() -> Self {
        Self {
            vals: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlattenedLeaf {
    pub leaf_index: u64,
    pub leaf_value: HexBytes,
}

lazy_static! {
    pub static ref FLATTENED_LEAF_HEAD: FlattenedLeaf = FlattenedLeaf {
        leaf_index: 0,
        leaf_value: LeafOpening::head().hval.0.to_vec().into(),
    };
    pub static ref FLATTENED_LEAF_TAIL: FlattenedLeaf = FlattenedLeaf {
        leaf_index: 1,
        leaf_value: LeafOpening::tail().hval.0.to_vec().into(),
    };
}

impl FlattenedLeaf {
    pub fn head() -> &'static Self {
        &FLATTENED_LEAF_HEAD
    }

    pub fn tail() -> &'static Self {
        &FLATTENED_LEAF_TAIL
    }

    pub fn new(leaf_index: u64, leaf_value: HexBytes) -> Self {
        Self {
            leaf_index,
            leaf_value,
        }
    }

    pub fn leaf_path(&self) -> [u8; ZK_TRIE_DEPTH + 2] {
        utils::get_leaf_path(self.leaf_index)
    }
}

impl MemStore {
    pub fn new() -> Self {
        Self {
            codes: BTreeMap::new(),
            nodes: BTreeMap::new(),
            index: LevelMap::new(),
            staging: BTreeMap::new(),
        }
    }

    pub fn from_traces(traces: &[Trace]) -> Result<Self, Error> {
        Ok(Self {
            codes: BTreeMap::new(),
            nodes: trace_nodes(traces),
            index: LevelMap::from_traces(traces)?,
            staging: BTreeMap::new(),
        })
    }

    pub fn add_codes(&mut self, codes: Vec<HexBytes>) {
        for code in codes {
            let hash = keccak_hash(&code);
            self.codes.insert(hash.into(), Arc::new(code));
        }
    }

    pub fn add_proof(
        &mut self,
        prefix: u64,
        leaf_index: u64,
        hkey: SH256,
        value: Option<&[u8]>,
        mut siblings: &[HexBytes],
    ) -> Result<SH256, Error> {
        // siblings:
        //   root
        //   subProof
        //   leaf

        let trie_path = utils::get_leaf_path(leaf_index);
        assert_eq!(siblings.len(), trie_path.len());
        let root = Node::new(NodeValue::parse_root(&siblings[0])?);

        let mut out = Vec::new();
        let leaf_value = &siblings[siblings.len() - 1];
        siblings = &siblings[..siblings.len() - 1];
        let mut leaf = Node::new(NodeValue::parse_leaf(
            vec![trie_path[trie_path.len() - 1]].into(),
            leaf_value.clone(),
        ));
        let mut leaf_hash = *leaf.hash();

        let sibling_leaf_idx = siblings.len() - 1;
        for (idx, sibling_bytes) in siblings.into_iter().enumerate().rev() {
            if idx == 0 {
                break;
            }
            let sibling = Node::new(if idx == sibling_leaf_idx {
                NodeValue::parse_leaf(
                    vec![trie_path[trie_path.len() - 1]].into(),
                    sibling_bytes.clone(),
                )
            } else {
                NodeValue::parse_branch(&sibling_bytes)?
            });
            let sibling_hash = *sibling.hash();
            leaf = Node::raw_branch_auto(trie_path[idx], leaf_hash, sibling_hash);
            leaf_hash = *leaf.hash();
            out.push(sibling);
            out.push(leaf);
        }

        if leaf_hash != root.raw().branch().unwrap().right {
            return Err(Error::InvalidProof);
        }
        let root_hash = *root.hash();
        out.push(root);

        for node in out {
            self.nodes.insert(*node.hash(), Arc::new(node));
        }
        self.index
            .vals
            .entry(root_hash)
            .or_insert_with(|| BTreeMap::new())
            .entry((prefix, hkey))
            .or_insert_with(|| KeyRange {
                left_index: 0,
                center: Some(FlattenedLeaf {
                    leaf_index,
                    leaf_value: value.unwrap().into(),
                }),
                right_index: 0,
            });
        Ok(root_hash)
    }

    fn get_code(&mut self, hash: &eth_types::SH256) -> Option<Arc<HexBytes>> {
        self.codes.get(hash).cloned()
    }

    fn get_node(&self, key: &SH256) -> Result<Option<Arc<Node>>, Error> {
        match EMPTY_TRIE_NODE.get(key) {
            Some(n) => return Ok(Some(n.clone())),
            None => {}
        };
        Ok(self.nodes.get(key).map(|n| n.clone()))
    }

    fn update_node(&mut self, key: SH256, node: Node) -> Result<Arc<Node>, Error> {
        let node = Arc::new(node);
        self.nodes.insert(key, node.clone());
        Ok(node)
    }

    fn update_index(&mut self, k: SH256, v: FlattenedLeaf) {
        self.staging.insert(k, v);
    }

    fn get_nearest_keys(&self, prefix: u64, root: &SH256, k: &SH256) -> KeyRange {
        match self.index.vals.get(root) {
            Some(map) => match map.get(&(prefix, *k)) {
                Some(r) => r.clone(),
                None => {
                    dbg!(&map);
                    unreachable!("index not found at root({:?}): key={:?}", root, (prefix, k))
                }
            },
            None => unreachable!("unknown root: {:?}", root),
        }
    }

    fn remove_index(&mut self, k: &SH256) {
        self.staging.remove(k);
    }
}

fn trace_nodes(traces: &[Trace]) -> BTreeMap<SH256, Arc<Node>> {
    let mut n = BTreeMap::new();
    for trace in traces {
        let (next_free_node, sub_root) = trace.old_state();
        let next_free_node = Node::next_free_node(next_free_node);
        let root = Node::raw_branch(*next_free_node.hash(), sub_root);
        n.insert(*root.hash(), Arc::new(root));
        n.insert(*next_free_node.hash(), Arc::new(next_free_node));
        for node in trace.nodes() {
            n.insert(*node.hash(), Arc::new(node));
        }
    }
    n
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyRange {
    pub left_index: u64,
    pub center: Option<FlattenedLeaf>,
    pub right_index: u64,
}

impl KeyRange {
    pub fn left_path(&self) -> [u8; ZK_TRIE_DEPTH + 2] {
        utils::get_leaf_path(self.left_index)
    }
    pub fn right_path(&self) -> [u8; ZK_TRIE_DEPTH + 2] {
        utils::get_leaf_path(self.right_index)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // #[test]
    // fn test_range() {
    //     let mut map = BTreeMap::new();

    //     for i in (0u8..20).step_by(2) {
    //         map.insert(i, ());
    //     }
    //     assert_eq!(
    //         Range::search(&map, &9),
    //         Some(Range {
    //             left: (8, ()),
    //             center: None,
    //             right: (10, ())
    //         })
    //     );

    //     assert_eq!(
    //         Range::search(&map, &10),
    //         Some(Range {
    //             left: (8, ()),
    //             center: Some((10, ())),
    //             right: (12, ())
    //         })
    //     );

    //     map.clear();
    //     assert_eq!(Range::search(&map, &10), None);

    //     map.insert(5, ());
    //     assert_eq!(
    //         Range::search(&map, &10),
    //         Some(Range {
    //             left: (5, ()),
    //             center: None,
    //             right: (5, ())
    //         })
    //     );
    //     assert_eq!(
    //         Range::search(&map, &2),
    //         Some(Range {
    //             left: (5, ()),
    //             center: None,
    //             right: (5, ())
    //         })
    //     );
    // }
}
