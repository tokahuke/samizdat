use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::Hash;

struct StreamCipher {
    current_hash: Hash,
    idx: usize,
}

impl Iterator for StreamCipher {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        let ret = if self.idx < 28 {
            self.current_hash[self.idx]
        } else {
            self.current_hash = Hash::build(&self.current_hash);
            self.idx = 0;
            self.current_hash[0]
        };

        Some(ret)
    }
}

impl StreamCipher {
    fn new(hash: Hash) -> StreamCipher {
        StreamCipher {
            current_hash: hash,
            idx: 0,
        }
    }

    fn xor(&mut self, slice: &mut [u8]) {
        for (confusing, byte) in self.zip(slice) {
            *byte ^= confusing;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub socket_addr: SocketAddr,
    /// A short which is always zero, for validation purposes.
    validation: u16,
}

impl Message {
    pub fn new(socket_addr: SocketAddr) -> Message {
        Message {
            socket_addr,
            validation: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRiddle {
    pub timestamp: i64,
    pub rand: Hash,
    pub hash: Hash,
}

impl ContentRiddle {
    pub fn new(content_hash: &Hash) -> ContentRiddle {
        let timestamp = chrono::Utc::now().timestamp();
        let rand = Hash::rand();
        let hash = content_hash.rehash(&timestamp.into()).rehash(&rand);

        ContentRiddle {
            timestamp,
            rand,
            hash,
        }
    }

    pub fn riddle_for_message(&self, message: &Message) -> MessageRiddle {
        // TODO: ooops! leaks IP type. Problem?
        let mut serialized = bincode::serialize(&message).expect("can always serialize");

        StreamCipher::new(self.hash).xor(&mut serialized);

        MessageRiddle {
            timestamp: self.timestamp,
            rand: self.rand,
            masked: serialized,
        }
    }

    pub fn resolves(&self, hash: &Hash) -> bool {
        hash.rehash(&self.timestamp.into()).rehash(&self.rand) == self.hash
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageRiddle {
    pub timestamp: i64,
    pub rand: Hash,
    pub masked: Vec<u8>,
}

impl MessageRiddle {
    pub fn resolve(&self, content_hash: &Hash) -> Option<Message> {
        let key = content_hash.rehash(&self.timestamp.into()).rehash(&self.rand);
        let mut serialized = self.masked.clone();

        StreamCipher::new(key).xor(&mut serialized);

        bincode::deserialize(&serialized)
            .ok()
            .and_then(|message: Message| {
                if message.validation == 0 {
                    Some(message)
                } else {
                    None
                }
            })
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     fn test_propose_resolve_message_riddle() {
//         let hash = Hash::rand();
//         let content_riddle = ContentRiddle::new(&hash);

//     }
// }
