use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::{HexBytes, SH256};
use lazy_static::lazy_static;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    trie_hash, utils, Database, Error, LeafOpening, Node, NodeValue, Trace, TraceProof,
    EMPTY_TRIE_NODE, ZK_TRIE_DEPTH,
};

#[derive(Debug)]
pub struct MemStore {
    prefix: u64,
    nodes: BTreeMap<SH256, Arc<Node>>,
    // index: BTreeMap<SH256, FlattenedLeaf>,
    // traces: BTreeMap<SH256, Vec<Trace>>,
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

// fn to_range(p: &TraceProof, l: &LeafOpening) -> (SH256, FlattenedLeaf) {
//     (
//         l.hkey,
//         FlattenedLeaf {
//             leaf_index: p.leaf_index,
//             leaf_value: l.hval.0.to_vec(),
//         },
//     )
// }

impl LevelMap {
    pub fn from_traces(prefix: u64, traces: &[Trace]) -> Result<Self, Error> {
        let mut idx = 0;
        let mut base = LevelMap::new();
        loop {
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
        Ok(base)
    }

    pub fn new() -> Self {
        Self {
            vals: BTreeMap::new(),
        }
    }

    // pub fn get_level(&self, root: &SH256) -> Option<&LevelMap> {
    //     let mut map = self;
    //     loop {
    //         if &map.root == root {
    //             return Some(map);
    //         }
    //         map = match &map.down {
    //             Some(n) => &n,
    //             None => return None,
    //         }
    //     }
    // }
}

// impl std::ops::Deref for LevelMap {
//     type Target = BTreeMap<SH256, KeyRange>;
//     fn deref(&self) -> &Self::Target {
//         &self.vals
//     }
// }

// impl std::ops::DerefMut for LevelMap {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         &mut self.vals
//     }
// }

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
            prefix: u64::max_value(),
            nodes: BTreeMap::new(),
            index: LevelMap::new(),
            staging: BTreeMap::new(),
            // traces: BTreeMap::new(),
        }
    }

    pub fn from_traces(traces: &[Trace]) -> Result<Self, Error> {
        let prefix = u64::max_value();
        Ok(Self {
            codes: BTreeMap::new(),
            prefix,
            nodes: trace_nodes(traces),
            index: LevelMap::from_traces(prefix, traces)?,
            staging: BTreeMap::new(),
        })
    }

    pub fn with_prefix(&mut self, prefix: u64) -> MemStoreSlot {
        MemStoreSlot { prefix, db: self }
    }

    pub fn add_codes(&mut self, codes: Vec<HexBytes>) {
        for code in codes {
            let hash = keccak_hash(&code);
            self.codes.insert(hash.into(), Arc::new(code));
        }
    }

    pub fn add_proof(
        &mut self,
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
            .insert(
                (self.prefix, hkey),
                KeyRange {
                    left_index: 0,
                    center: Some(FlattenedLeaf {
                        leaf_index,
                        leaf_value: value.unwrap().into(),
                    }),
                    right_index: 0,
                },
            );
        Ok(root_hash)
    }
}

fn trace_nodes(traces: &[Trace]) -> BTreeMap<SH256, Arc<Node>> {
    let mut n = BTreeMap::new();
    for trace in traces {
        for node in trace.nodes() {
            n.insert(*node.hash(), Arc::new(node));
        }
    }
    n
}

impl Database for MemStore {
    type Node = Node;
    fn get_code(&mut self, hash: &eth_types::SH256) -> Option<Arc<HexBytes>> {
        self.codes.get(hash).cloned()
    }
    
    fn get_node(&self, key: &SH256) -> Result<Option<Arc<Self::Node>>, Error> {
        match EMPTY_TRIE_NODE.get(key) {
            Some(n) => return Ok(Some(n.clone())),
            None => {}
        };
        Ok(self.nodes.get(key).map(|n| n.clone()))
    }

    fn update_node(&mut self, key: SH256, node: Self::Node) -> Result<Arc<Self::Node>, Error> {
        let node = Arc::new(node);
        self.nodes.insert(key, node.clone());
        Ok(node)
    }

    fn update_index(&mut self, k: SH256, v: FlattenedLeaf) {
        self.staging.insert(k, v);
    }

    fn get_nearest_keys(&self, root: &SH256, k: &SH256) -> KeyRange {
        match self.index.vals.get(root) {
            Some(map) => match map.get(&(self.prefix, *k)) {
                Some(r) => r.clone(),
                None => {
                    dbg!(&map);
                    unreachable!("should exist: {:?}", k)
                }
            },
            None => unreachable!("unknown root: {:?}", root),
        }
    }

    fn remove_index(&mut self, k: &SH256) {
        self.staging.remove(k);
    }
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
    // pub fn search_with_default<F: FnOnce() -> (K, V)>(
    //     map: &BTreeMap<SH256, FlattenedLeaf>,
    //     key: &K,
    //     default: F,
    // ) -> Self {
    //     match Self::search(map, key) {
    //         Some(n) => n,
    //         None => {
    //             let default = default();
    //             Range {
    //                 left: default.clone(),
    //                 center: None,
    //                 right: default.clone(),
    //             }
    //         }
    //     }
    // }

    // pub fn search(map: &BTreeMap<K, V>, key: &K) -> Option<Self> {
    //     if map.len() == 0 {
    //         return None;
    //     }
    //     let mut lower_range = map.range(..key);
    //     let mut upper_range = map.range(key..);
    //     let mut center = None;
    //     let mut left = lower_range.next_back();
    //     let mut right = upper_range.next();
    //     if let Some((k, v)) = right {
    //         if k == key {
    //             center = Some((k.clone(), v.clone()));
    //             right = upper_range.next();
    //         }
    //     }
    //     let left = left
    //         .map(|(k, v)| (k.clone(), v.clone()))
    //         .unwrap_or_else(|| {
    //             let (k, v) = map.iter().next().unwrap();
    //             (k.clone(), v.clone())
    //         });
    //     let right = right
    //         .map(|(k, v)| (k.clone(), v.clone()))
    //         .unwrap_or_else(|| {
    //             let (k, v) = map.iter().next_back().unwrap();
    //             (k.clone(), v.clone())
    //         });
    //     Some(Self {
    //         left,
    //         center,
    //         right,
    //     })
    // }
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
