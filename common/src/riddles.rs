use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

use crate::cipher::{OpaqueEncrypted, TransferCipher};
use crate::Hash;

#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
struct Message<T> {
    pub payload: T,
    /// A short which is always zero, for validation purposes.
    validation: u16,
}

impl<T> Message<T> {
    pub fn new(payload: T) -> Message<T> {
        Message {
            payload,
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

    pub fn riddle_for<T>(&self, message: T) -> MessageRiddle
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        // TODO: ooops! leaks message length (i.e., IP type, etc...). Problem?
        // Need padding!
        let masked =
            TransferCipher::new(&self.hash, &self.rand).encrypt_opaque(&Message::new(message));

        MessageRiddle {
            timestamp: self.timestamp,
            rand: self.rand,
            masked,
        }
    }

    pub fn resolves(&self, hash: &Hash) -> bool {
        hash.rehash(&self.timestamp.into()).rehash(&self.rand) == self.hash
    }
}

#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
pub struct MessageRiddle {
    pub timestamp: i64,
    pub rand: Hash,
    pub masked: OpaqueEncrypted,
}

impl MessageRiddle {
    pub fn resolve<T>(&self, content_hash: &Hash) -> Option<T>
    where
        T: for<'a> Deserialize<'a>,
    {
        let key = content_hash
            .rehash(&self.timestamp.into())
            .rehash(&self.rand);

        let unmasked = self
            .masked
            .clone()
            .decrypt_with(&TransferCipher::new(&key, &self.rand));

        unmasked.ok().and_then(|message: Message<T>| {
            if message.validation == 0 {
                Some(message.payload)
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
