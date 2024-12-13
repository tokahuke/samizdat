//! Asymmetric cryptography primitives for Samizdat.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as DeriveDeserialize, Serialize as DeriveSerialize};
use std::fmt::{self, Debug, Display};
use std::ops::Deref;
use std::str::FromStr;

use crate::Hash;

/// A signed piece of data. The data is to be serialized with bincode and the serialized
/// binary data is signed.
#[derive(Debug, Clone, DeriveSerialize, DeriveDeserialize)]
pub struct Signed<T> {
    /// The data to which the signature refers to.
    content: T,
    /// The signature for the data.
    signature: Signature,
}

impl<T> Signed<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    /// Create a new signature for some information, given a keypair.
    pub fn new(content: T, keypair: &SigningKey) -> Signed<T> {
        let signature = keypair.sign(&bincode::serialize(&content).expect("can serialize"));
        Signed { content, signature }
    }

    /// Retrieve the content of the signature.
    pub fn into_inner(self) -> T {
        self.content
    }

    /// Checks of the signature is valid under the supplied public key.
    pub fn verify(&self, public_key: &VerifyingKey) -> bool {
        public_key
            .verify_strict(
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

/// A private key.
#[derive(DeriveDeserialize, DeriveSerialize)]
#[serde(transparent)]
pub struct PrivateKey(ed25519_dalek::SecretKey);

impl FromStr for PrivateKey {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<PrivateKey, crate::Error> {
        Ok(PrivateKey(
            ed25519_dalek::SecretKey::try_from(base64_url::decode(s)?).map_err(|dec| {
                format!(
                    "Failed to deserialize secret key {s}: wrong key length, got {}",
                    dec.len()
                )
            })?,
        ))
    }
}

impl Display for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl AsRef<ed25519_dalek::SecretKey> for PrivateKey {
    fn as_ref(&self) -> &ed25519_dalek::SecretKey {
        &self.0
    }
}

impl From<ed25519_dalek::SecretKey> for PrivateKey {
    fn from(key: ed25519_dalek::SecretKey) -> PrivateKey {
        PrivateKey(key)
    }
}

impl From<PrivateKey> for ed25519_dalek::SecretKey {
    fn from(key: PrivateKey) -> ed25519_dalek::SecretKey {
        key.0
    }
}

impl From<PrivateKey> for ed25519_dalek::SigningKey {
    fn from(key: PrivateKey) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&key.0)
    }
}

/// A public key.
#[derive(Clone, PartialEq, Eq, DeriveDeserialize, DeriveSerialize)]
#[serde(transparent)]
pub struct Key(ed25519_dalek::VerifyingKey);

impl FromStr for Key {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Key, crate::Error> {
        let bytes = base64_url::decode(s)?;

        Ok(Key(ed25519_dalek::VerifyingKey::from_bytes(
            &bytes[..]
                .try_into()
                .map_err(|_| "Bad size for public key".to_string())?,
        )
        .map_err(|err| {
            format!("Failed to deserialize public key {s}: {err}")
        })?))
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl AsRef<ed25519_dalek::VerifyingKey> for Key {
    fn as_ref(&self) -> &ed25519_dalek::VerifyingKey {
        &self.0
    }
}

impl From<ed25519_dalek::VerifyingKey> for Key {
    fn from(key: ed25519_dalek::VerifyingKey) -> Key {
        Key(key)
    }
}

impl Key {
    /// Creates a new public key from a raw public key.
    pub fn new(key: ed25519_dalek::VerifyingKey) -> Key {
        Key(key)
    }

    /// Retrieve the raw public key from the public key.
    pub fn into_inner(self) -> ed25519_dalek::VerifyingKey {
        self.0
    }

    /// Gets the hash of this public key.
    pub fn hash(&self) -> Hash {
        Hash::from_bytes(self.0.as_bytes())
    }

    /// Deserializes a public key from binary data.
    pub fn from_bytes(bytes: &[u8]) -> Result<Key, crate::Error> {
        Ok(Key(ed25519_dalek::VerifyingKey::from_bytes(
            bytes
                .try_into()
                .map_err(|_| "Bad size for public key".to_string())?,
        )
        .map_err(|err| format!("bad public key: {}", err))?))
    }

    /// Retrieves the binary representation of this public key.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}
