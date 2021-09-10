use ed25519_dalek::{Keypair, PublicKey, Signature, Signer, Verifier};
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as DeriveDeserialize, Serialize as DeriveSerialize};
use std::fmt::{self, Display};
use std::ops::Deref;
use std::str::FromStr;

use crate::Hash;

#[derive(Debug, DeriveSerialize, DeriveDeserialize)]
pub struct Signed<T> {
    content: T,
    signature: Signature,
}

impl<T> Signed<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    pub fn new(content: T, keypair: &Keypair) -> Signed<T> {
        let signature = keypair.sign(&bincode::serialize(&content).expect("can serialize"));
        Signed { content, signature }
    }

    pub fn into_inner(self) -> T {
        self.content
    }

    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key
            .verify(
                &bincode::serialize(&self.content).expect("can serialize"),
                &self.signature,
            )
            .is_ok()
    }
}

impl<T> Deref for Signed<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq, DeriveDeserialize, DeriveSerialize)]
pub struct Key(ed25519_dalek::PublicKey);

impl FromStr for Key {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Key, crate::Error> {
        // TODO: oops! need to process this error.
        Ok(Key(ed25519_dalek::PublicKey::from_bytes(
            &base64_url::decode(s)?,
        )
        .expect("bad key")))
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl AsRef<ed25519_dalek::PublicKey> for Key {
    fn as_ref(&self) -> &ed25519_dalek::PublicKey {
        &self.0
    }
}

impl From<ed25519_dalek::PublicKey> for Key {
    fn from(key: ed25519_dalek::PublicKey) -> Key {
        Key(key)
    }
}

impl Key {
    pub fn new(key: ed25519_dalek::PublicKey) -> Key {
        Key(key)
    }

    pub fn into_inner(self) -> ed25519_dalek::PublicKey {
        self.0
    }

    pub fn hash(&self) -> Hash {
        Hash::build(self.0.as_bytes())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Key, crate::Error> {
        Ok(Key(ed25519_dalek::PublicKey::from_bytes(bytes)
            .map_err(|err| format!("bad public key: {}", err))?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}