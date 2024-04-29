use std::{io::Write, prelude::v1::*};

use crate::{prefix_len, utils, Error, FlattenedLeaf, HashOrNode, KeyRange, Node, NodeValue};
use eth_types::{HexBytes, SH160, SH256};
use std::sync::Arc;

pub const ZK_TRIE_DEPTH: usize = 40;

pub trait Database {
    type Node;
    fn get_node(&self, key: &SH256) -> Result<Option<Arc<Self::Node>>, Error>;
    fn update_node(&mut self, key: SH256, node: Self::Node) -> Result<Arc<Self::Node>, Error>;
    fn get_nearest_keys(&self, root: &SH256, k: &SH256) -> KeyRange;
    fn update_index(&mut self, k: SH256, v: FlattenedLeaf);
    fn remove_index(&mut self, k: &SH256);
    fn get_code(&mut self, hash: &SH256) -> Option<Arc<HexBytes>>;
}

pub struct NodeData {
    node: Arc<Node>,
}

#[derive(Debug)]
pub struct SpareMerkleTrie {
    root: SH256,
}

impl SpareMerkleTrie {
    pub fn new(root_hash: SH256) -> Self {

        Ok(Self { root })
    }

    pub fn new_from_sub_root(next_free_node: u64, sub_root: SH256) -> Self {
        Self {
            root: Arc::new(Node::top_node(next_free_node, sub_root)),
        }
    }

    pub fn root_hash(&self) -> &SH256 {
        self.root.hash()
    }

    pub fn sub_root_hash(&self) -> &SH256 {
        let branch_node = self.root.raw().branch().unwrap();
        &branch_node.right
    }

    pub fn next_free_node(&self) -> Option<u64> {
        let branch_node = self.root.raw().branch()?;
        match &branch_node.left {
            HashOrNode::Node(n) => n.value().map(utils::parse_node_index),
            _ => None,
        }
    }

    pub fn set_next_free_node(&mut self, free: u64) -> Result<(), Error> {
        let branch_node = self.root.raw().branch().unwrap();
        self.root = Arc::new(
            branch_node
                .new_replace(0, &Arc::new(Node::next_free_node(free)))
                .into(),
        );
        Ok(())
    }

    pub fn root_node<D: Database<Node = Node>>(&self, db: &D) -> Result<Option<Arc<Node>>, Error> {
        db.get_node(self.root.hash())
    }

    pub fn write_to<D: Database<Node = Node>>(&self, db: &D, f: &mut impl Write) {
        Self::print(db, self.root.hash(), 0, f).unwrap()
    }

    fn print<D: Database<Node = Node>>(
        db: &D,
        root: &SH256,
        lvl: usize,
        f: &mut impl Write,
    ) -> std::io::Result<()> {
        let node = db.get_node(root).unwrap().unwrap();
        if node.is_empty_node() {
            return writeln!(f, "{}{}(empty)", " ".repeat(lvl), node.raw().ty());
        }
        writeln!(
            f,
            "{}{}({:?}) = {:?}",
            "|".repeat(lvl) + "-",
            node.raw().ty(),
            node.hash(),
            node.value(),
        )?;
        match node.raw() {
            NodeValue::Branch(n) => {
                Self::print(db, n.left_hash(), lvl + 1, f)?;
                Self::print(db, &n.right, lvl + 1, f)?;
            }
            _ => (),
        }
        Ok(())
    }

    pub fn commit<D: Database<Node = Node>>(&self, db: &mut D) {}

    pub fn put<D: Database<Node = Node>>(
        &mut self,
        db: &mut D,
        path: &[u8],
        value: Vec<u8>,
    ) -> Result<(), Error> {
        self.root = self.add_leaf(db, 0, (&self.root).into(), path, value)?;
        Ok(())
    }

    pub fn remove<D: Database<Node = Node>>(
        &mut self,
        db: &mut D,
        path: &[u8],
    ) -> Result<(), Error> {
        self.root = self.remove_leaf(db, 0, (&self.root).into(), path)?;
        Ok(())
    }

    fn remove_leaf<D: Database<Node = Node>>(
        &mut self,
        db: &mut D,
        lvl: usize,
        current: HashOrNode,
        path: &[u8],
    ) -> Result<Arc<Node>, Error> {
        if lvl >= ZK_TRIE_DEPTH + 2 {
            return Err(Error::ReachedMaxLevel);
        }
        let n = match current.expand(db)? {
            Some(n) => n,
            None => return Err(Error::NodeNotFound(lvl, *current.hash())),
        };
        Ok(match n.raw() {
            NodeValue::Branch(branch) => {
                let child = branch.child(path[lvl]).into();
                let updated_child = self.remove_leaf(db, lvl + 1, child, path)?;
                self.db_add(db, branch.new_replace(path[lvl], &updated_child).into())?
            }
            NodeValue::Leaf(leaf) => Node::empty_leaf(),
            NodeValue::EmptyLeaf => Node::empty_leaf(),
            NodeValue::NextFree(node) => Node::empty_leaf(),
        })
    }

    fn add_leaf<D: Database<Node = Node>>(
        &self,
        db: &mut D,
        lvl: usize,
        current: HashOrNode,
        path: &[u8],
        value: Vec<u8>,
    ) -> Result<Arc<Node>, Error> {
        if lvl >= ZK_TRIE_DEPTH + 2 {
            return Err(Error::ReachedMaxLevel);
        }
        let n = match current.expand(db)? {
            Some(n) => n,
            None => return Err(Error::NodeNotFound(lvl, *current.hash())),
        };
        Ok(match n.raw() {
            NodeValue::Branch(branch) => {
                let child = branch.child(path[lvl]).into();
                let updated_child = self.add_leaf(db, lvl + 1, child, path, value)?;
                self.db_add(db, branch.new_replace(path[lvl], &updated_child).into())?
            }
            NodeValue::Leaf(leaf) => {
                let common_path_len = prefix_len(&leaf.path, &path[lvl..]);
                if common_path_len == leaf.path.len() {
                    self.db_add(db, Node::leaf(path[lvl..].to_vec(), value))?
                } else {
                    return Err(Error::PathNotAllow);
                }
            }
            NodeValue::EmptyLeaf => self.db_add(db, Node::leaf(path[lvl..].to_vec(), value))?,
            NodeValue::NextFree(node) => {
                let common_path_len = prefix_len(&node.path, &path[lvl..]);
                if common_path_len == node.path.len() {
                    self.db_add(db, Node::leaf(path[lvl..].to_vec(), value))?
                } else {
                    unreachable!()
                }
            }
        })
    }

    fn db_add<D: Database<Node = Node>>(&self, db: &mut D, node: Node) -> Result<Arc<Node>, Error> {
        db.update_node(*node.hash(), node)
    }

    pub fn get_node<D: Database<Node = Node>>(
        &self,
        db: &D,
        path: &[u8],
    ) -> Result<Option<Arc<Node>>, Error> {
        let mut next_node = HashOrNode::Node(self.root.clone());
        for i in 0..(ZK_TRIE_DEPTH + 2) {
            let n = match next_node.expand(db)? {
                Some(n) => n,
                None => return Err(Error::NodeNotFound(i, *next_node.hash())),
            };

            match n.raw() {
                NodeValue::Branch(node) => {
                    if path[i] == 0 {
                        next_node = node.left_hash().into();
                    } else {
                        next_node = node.right.into();
                    }
                }
                NodeValue::EmptyLeaf => return Ok(None),
                NodeValue::Leaf(node) => {
                    if &node.path == &path[i..] {
                        return Ok(Some(n));
                    }
                    return Ok(None);
                }
                NodeValue::NextFree(node) => {
                    if &node.path == &path[i..] {
                        return Ok(Some(n));
                    }
                    unreachable!()
                }
            }
        }
        Err(Error::ReachedMaxLevel)
    }
}
