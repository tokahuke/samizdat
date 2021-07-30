use std::str::FromStr;
use std::fmt::{self, Display};
use std::convert::TryInto;
use std::ops::Deref;
use sha3::{Digest, Sha3_224};

#[derive(Debug)]
pub struct Hash(pub [u8; 28]);

impl FromStr for Hash {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Hash, crate::Error> {
        Ok(Hash(base64_url::decode(s)?.try_into().map_err(
            |e: Vec<_>| format!("expected 64 bytes; got {}", e.len()),
        )?))
    }
}

impl Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl Deref for Hash {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Hash {
    /// # Panics
    ///
    /// If the received slice does not have the correct length of 64 bytes.
    pub fn new(x: impl AsRef<[u8]>) -> Hash {
        Hash(x.as_ref().try_into().expect("bad hash value"))
    }

    pub fn build(thing: impl AsRef<[u8]>) -> Hash {
        Hash::new(Sha3_224::digest(thing.as_ref()))
    }
}
