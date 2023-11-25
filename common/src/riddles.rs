//! Riddles are cryptographic challenges use to test whether an agent knows a given
//! information without revealing the information itself.

use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

use crate::cipher::{OpaqueEncrypted, TransferCipher};
use crate::Hash;

/// A message that can be passed around and only decoded by who knows the secret solution
/// of a riddle.
#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
struct Message<T> {
    /// The payload of this message.
    pub payload: T,
    /// A short which is always zero, for validation purposes.
    validation: u16,
}

impl<T> Message<T> {
    /// Creates a new message, with a given payload.
    pub fn new(payload: T) -> Message<T> {
        Message {
            payload,
            validation: 0,
        }
    }
}

/// Riddles are cryptographic challenges use to test whether an agent knows a given
/// information without revealing the information itself.
///
/// More specificly, a riddle is a cryptographic riddle for a hidden value. It basically
/// asks: which [`struct@Hash`] `h` has `H(h || nonce)` equal to `X`? If `H` is a sound
/// hashing function, then the only ones who can solve this riddle are the ones who know
/// `h`.
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

    /// Tests whether the given hash solves the supplied riddle.
    pub fn resolves(&self, hash: &Hash) -> bool {
        hash.rehash(&self.rand) == self.hash
    }
}

/// A message riddle works just like a riddle, except that a payload is also sent. This
/// payload is ciphered using the response to the riddle that generated this message
/// riddle.
#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
pub struct MessageRiddle {
    /// The random initialization of the symmetric cipher.
    pub rand: Hash,
    /// The encrypted contents of this message riddle.
    pub masked: OpaqueEncrypted,
}

impl MessageRiddle {
    /// Tries to resolve the message riddle, given a proposed hash. If the proposed hash
    /// does not solve the message riddle, [`None`] is returned.
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

/// A hint on the solution of a riddle. This gives the prefix of the solution, up to a given
/// length.
#[derive(Debug, Clone, SerdeSerialize, SerdeDeserialize)]
pub struct Hint {
    /// The prefix of the solution of the riddle.
    prefix: Hash,
    /// The length in bytes of the prefix. Everything after this length in the `prefix` hash is
    /// ignored.
    length: u8,
}

impl Hint {
    pub fn new(content_hash: Hash, length: usize) -> Hint {
        assert!(length < 256, "Length has to fit in a byte");
        let mut prefix = Hash::zero();
        for i in 0..length {
            prefix.0[i] = content_hash.0[i];
        }
        Hint {
            prefix,
            length: length as u8,
        }
    }

    pub fn prefix(&self) -> &[u8] {
        &self.prefix.0[..(self.length as usize)]
    }

    pub fn len(&self) -> usize {
        self.length as usize
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
