use std::prelude::v1::*;

use crypto::keccak_hash;
use eth_types::KeccakHasher;
use lazy_static::lazy_static;

use crate::{fr::BYTES, Fr};

pub(crate) const MIMC_NB_ROUNDS: usize = 62;
const SEED: &str = "seed";
pub const BLOCK_SIZE: usize = BYTES;
use crate::ff::{Field, PrimeField};

lazy_static! {
    pub(crate) static ref MIMC_CONSTANTS: [Fr; MIMC_NB_ROUNDS] = init_constants();
}

fn init_constants() -> [Fr; MIMC_NB_ROUNDS] {
    let mut mimc = [Fr::default(); MIMC_NB_ROUNDS];

    let mut rnd = keccak_hash(SEED.as_bytes());
    rnd = keccak_hash(&rnd);
    for (i, item) in mimc.iter_mut().enumerate() {
        *item = Fr::from_be(&rnd).unwrap();
        rnd = keccak_hash(&rnd);
    }

    mimc
}