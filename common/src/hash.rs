use sha3::{Digest, Sha3_224};
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Display};
use std::ops::Deref;
use std::str::FromStr;

use crate::ContentRiddle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

impl<'a> TryFrom<&'a [u8]> for Hash {
    type Error = crate::Error;
    fn try_from(slice: &'a [u8]) -> Result<Hash, crate::Error> {
        if slice.len() != 28 {
            Err(crate::Error::BadHashLength(slice.len()))
        } else {
            Ok(Hash(slice.try_into().expect("aleady checked")))
        }
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

    pub fn rehash(&self, rand: [u8; 28]) -> Hash {
        Hash::build([rand, self.0].concat())
    }

    pub fn gen_riddle(&self) -> ContentRiddle {
        let mut rand = [0; 28];
        getrandom::getrandom(&mut rand).expect("getrandom failed");
        let hash = self.rehash(rand);

        ContentRiddle { rand, hash: hash.0 }
    }
}
