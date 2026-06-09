//! Common utilities and types used across the Samizdat project.

pub extern crate quinn;
pub extern crate rustls;

pub mod address;
pub mod blockchain;
pub mod cipher;
pub mod db;
pub mod encoding;
pub mod heap_entry;
pub mod identity;
pub mod keyed_channel;
pub mod logger;
pub mod quic;
pub mod rpc;
pub mod transport;

mod error;
mod hash;
mod patricia_map;
mod pki;
mod riddles;
pub use error::Error;
pub use hash::{HASH_LEN, Hash, InclusionProof, MerkleTree};
pub use patricia_map::{PatriciaMap, PatriciaProof};
pub use pki::{Key, PrivateKey, Signed};
pub use riddles::{Hint, MessageRiddle, Riddle};

use rand::SeedableRng;
use rand_chacha::ChaChaRng;

/// A cryptographically secure PRNG, seeded with 32 bytes from the OS RNG
/// (the full state of `ChaChaRng`).
pub fn csprng() -> ChaChaRng {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).expect("getrandom failed");
    ChaChaRng::from_seed(seed)
}
