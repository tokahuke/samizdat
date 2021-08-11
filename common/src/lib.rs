#![feature(option_get_or_insert_default)]

pub mod logger;
pub mod quic;
pub mod rpc;

mod error;
mod hash;
mod riddles;
mod transport;
//mod merkle_map;
mod patricia_map;

pub use error::Error;
pub use hash::{Hash, InclusionProof, MerkleTree};
pub use riddles::{ContentRiddle, Message, MessageRiddle};
pub use transport::BincodeOverQuic;
//pub use merkle_map::MerkleMap;
