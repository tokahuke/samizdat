pub mod cipher;
pub mod heap_entry;
pub mod logger;
pub mod pow;
pub mod quic;
pub mod rpc;

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
pub use pki::{Key, PrivateKey, Signed};
pub use riddles::{MessageRiddle, Riddle};
pub use transport::BincodeOverQuic;

use rand::SeedableRng;
use rand_chacha::ChaChaRng;

pub fn csprng() -> ChaChaRng {
    let mut seed = [0; 8];
    getrandom::getrandom(&mut seed).expect("getrandom failed");
    ChaChaRng::seed_from_u64(u64::from_le_bytes(seed))
}
