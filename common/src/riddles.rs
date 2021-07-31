use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::Hash;

struct StreamCipher {
    current_hash: [u8; 28],
    idx: usize,
}

impl Iterator for StreamCipher {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        let ret = if self.idx < 28 {
            self.current_hash[self.idx]
        } else {
            self.current_hash = Hash::build(&self.current_hash).0;
            self.idx = 0;
            self.current_hash[0]
        };

        Some(ret)
    }
}

impl StreamCipher {
    fn new(hash: [u8; 28]) -> StreamCipher {
        StreamCipher {
            current_hash: hash,
            idx: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AddressMessage {
    socket_addr: SocketAddr,
    validation: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRiddle {
    pub rand: [u8; 28],
    pub hash: [u8; 28],
}

impl ContentRiddle {
    pub fn riddle_for_location(&self, socket_addr: SocketAddr) -> LocationRiddle {
        // TODO: ooops! leaks IP type. Problem?
        let mut serialized = bincode::serialize(&AddressMessage {
            socket_addr,
            validation: 0,
        })
        .expect("can always serialize");

        for (confusing, byte) in StreamCipher::new(self.hash.clone()).zip(&mut serialized) {
            *byte ^= confusing;
        }

        LocationRiddle {
            rand: self.rand.clone(),
            masked: serialized,
        }
    }

    pub fn resolves(&self, hash: &Hash) -> bool {
        hash.rehash(self.rand).0 == self.hash
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocationRiddle {
    pub rand: [u8; 28],
    pub masked: Vec<u8>,
}

impl LocationRiddle {
    pub fn resolve(&self, hash: &Hash) -> Option<SocketAddr> {
        let key = hash.rehash(self.rand);
        let mut serialized = self.masked.clone();

        for (confusing, byte) in StreamCipher::new(key.0).zip(&mut serialized) {
            *byte ^= confusing;
        }

        bincode::deserialize(&serialized)
            .ok()
            .and_then(|message: AddressMessage| {
                if message.validation == 0 {
                    Some(message.socket_addr)
                } else {
                    None
                }
            })
    }
}
