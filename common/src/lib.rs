pub mod cipher;
pub mod logger;
pub mod quic;
pub mod rpc;
pub mod heap_entry;

mod channel_address;
mod error;
mod hash;
mod patricia_map;
mod pki;
mod riddles;
mod transport;

pub use channel_address::ChannelAddr;
pub use error::Error;
pub use hash::{Hash, InclusionProof, MerkleTree};
pub use patricia_map::{PatriciaMap, PatriciaProof};
pub use pki::{Key, Signed};
pub use riddles::{ContentRiddle, MessageRiddle};
pub use transport::BincodeOverQuic;

use rand::{CryptoRng, RngCore, SeedableRng};
use rand_chacha::ChaChaRng;

pub fn csprng() -> impl CryptoRng + RngCore {
    let mut seed = [0; 8];
    getrandom::getrandom(&mut seed).expect("getrandom failed");
    ChaChaRng::seed_from_u64(u64::from_le_bytes(seed))
}
