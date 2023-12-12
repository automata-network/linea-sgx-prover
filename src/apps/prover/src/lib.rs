#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "tstd")]
#[macro_use]
extern crate sgxlib as std;

mod apps;
pub use crate::apps::*;
mod types;
pub use types::*;