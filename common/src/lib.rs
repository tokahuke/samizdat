#![feature(ip)]

pub extern crate quinn;
pub extern crate rustls;

pub mod address;
pub mod blockchain;
pub mod cipher;
pub mod db;
pub mod heap_entry;
pub mod keyed_channel;
pub mod pow;
pub mod quic;
pub mod rpc;
pub mod transport;

mod error;
mod hash;
mod patricia_map;
mod pki;
mod riddles;

pub use error::Error;
pub use hash::{Hash, InclusionProof, MerkleTree, HASH_LEN};
pub use patricia_map::{PatriciaMap, PatriciaProof};
pub use pki::{Key, PrivateKey, Signed};
pub use riddles::{Hint, MessageRiddle, Riddle};

use rand::SeedableRng;
use rand_chacha::ChaChaRng;

/// Creates a cryptographically safe pseudo-random number generatros using [`getrandom`] to
/// generate the seed.
pub fn csprng() -> ChaChaRng {
    let mut seed = [0; 8];
    getrandom::getrandom(&mut seed).expect("getrandom failed");
    ChaChaRng::seed_from_u64(u64::from_le_bytes(seed))
}
