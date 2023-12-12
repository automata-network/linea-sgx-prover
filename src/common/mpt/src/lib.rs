#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(feature = "tstd")]
#[macro_use]
extern crate sgxlib as std;

mod trie_state;
pub use trie_state::*;

mod trie;
pub use trie::*;