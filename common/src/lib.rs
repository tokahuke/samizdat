pub mod cipher;
pub mod logger;
pub mod pki;
pub mod quic;
pub mod rpc;

mod error;
mod hash;
mod patricia_map;
mod riddles;
mod transport;

pub use error::Error;
pub use hash::{Hash, InclusionProof, MerkleTree};
pub use patricia_map::{PatriciaMap, PatriciaProof};
pub use riddles::{ContentRiddle, Message, MessageRiddle};
pub use transport::BincodeOverQuic;

use rand::{CryptoRng, RngCore, SeedableRng};
use rand_chacha::ChaChaRng;

pub fn csprng() -> impl CryptoRng + RngCore {
    let mut seed = [0; 8];
    getrandom::getrandom(&mut seed).expect("getrandom failed");
    ChaChaRng::seed_from_u64(u64::from_le_bytes(seed))
}
