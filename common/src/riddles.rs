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

/// A content riddle is a cryptographic riddle for a missing value. It basically asks: which
/// [`Hash`] `h` has `H(h || nonce)` equal to `X`? If `H` is a good hash, then
/// the only ones who can solve this riddle are the ones who know `h`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Riddle {
    /// The nonce used for the riddle.
    pub rand: Hash,
    /// The resulting hash of the rehasing operator (i.e., the value of `X`).
    pub hash: Hash,
}

impl Riddle {
    /// Creates a new [`Riddle`] from a given secret `content_hash`.
    pub fn new(content_hash: &Hash) -> Riddle {
        let rand = Hash::rand();
        let hash = content_hash.rehash(&rand);

        Riddle { rand, hash }
    }

    /// Creates a new [`Riddle`] from a given secret `content_hash` and a `nonce`.
    pub fn new_with_nonce(content_hash: &Hash, rand: Hash) -> Riddle {
        let hash = content_hash.rehash(&rand);
        Riddle { rand, hash }
    }

    /// Creates a message riddle for a message based on this hash.
    pub fn riddle_for<T>(&self, message: T) -> MessageRiddle
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        // TODO: ooops! leaks message length (i.e., IP type, etc...). Problem?
        // Need padding!
        let masked =
            TransferCipher::new(&self.hash, &self.rand).encrypt_opaque(&Message::new(message));

        MessageRiddle {
            rand: self.rand,
            masked,
        }
    }

    pub fn resolves(&self, hash: &Hash) -> bool {
        hash.rehash(&self.rand) == self.hash
    }
}

/// A message riddle works just like a riddle, except that a payload is also sent. This payload is
/// ciphered using the response to the riddle.
#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
pub struct MessageRiddle {
    pub rand: Hash,
    pub masked: OpaqueEncrypted,
}

impl MessageRiddle {
    pub fn resolve<T>(&self, content_hash: &Hash) -> Option<T>
    where
        T: for<'a> Deserialize<'a>,
    {
        let key = content_hash.rehash(&self.rand);

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
//         let content_riddle = Riddle::new(&hash);

//     }
// }
