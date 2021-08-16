use serde_derive::{Deserialize, Serialize};
use sha3::{Digest, Sha3_224};
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Debug, Display};
use std::ops::Deref;
use std::str::FromStr;

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

impl Debug for Hash {
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
            Ok(Hash(slice.try_into().expect("already checked")))
        }
    }
}

impl From<i64> for Hash {
    fn from(int: i64) -> Hash {
        let mut hash = Hash::default();
        let bytes = int.to_be_bytes();
        for i in 0..8 {
            hash.0[i] = bytes[i];
        }

        hash
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

    pub fn rand() -> Hash {
        let mut rand = [0; 28];
        getrandom::getrandom(&mut rand).expect("getrandom failed");

        Hash(rand)
    }

    pub fn rehash(&self, rand: &Hash) -> Hash {
        Hash::build([rand.0, self.0].concat())
    }

    pub fn is_proved_by(&self, root: &Hash, proof: &InclusionProof) -> bool {
        let mut mask = 1;
        let mut current = *self;

        for hash in proof.path.iter() {
            // The proof completes the left side if 1, else, right.
            current = if proof.index & mask != 0 {
                hash.rehash(&current)
            } else {
                current.rehash(hash)
            };

            if hash != &current {
                return false;
            }

            mask <<= 1;
        }

        &current == root
    }
}

pub struct MerkleTree {
    tree: Vec<Vec<Hash>>,
}

impl From<Vec<Hash>> for MerkleTree {
    fn from(vec: Vec<Hash>) -> MerkleTree {
        fn iterate_level(slice: &[Hash]) -> Vec<Hash> {
            slice
                .chunks(2)
                .map(|chunk| match chunk {
                    [single] => single.rehash(&Hash::default()),
                    [left, right] => left.rehash(&right),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        }

        MerkleTree {
            tree: std::iter::successors(Some(vec), |vec| {
                if vec.len() > 1 {
                    Some(iterate_level(&*vec))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        }
    }
}

impl MerkleTree {
    pub fn root(&self) -> Hash {
        self.tree[0][0]
    }

    pub fn len(&self) -> usize {
        self.tree.last().expect("not empty").len()
    }

    pub fn hashes(&self) -> &[Hash] {
        self.tree.last().expect("not empty")
    }

    /// Returns `None` if `index` out of range.
    pub fn proof_for(&self, index: usize) -> Option<InclusionProof> {
        if index > self.len() {
            return None;
        }

        let mut level_index = index;

        let path = self
            .tree
            .iter()
            .map(|level| {
                let sibling_index = level_index ^ 1;
                level_index >>= 1;

                if let Some(sibling) = level.get(sibling_index) {
                    *sibling
                } else {
                    // Incomplete level. Filling up.
                    Hash::default()
                }
            })
            .collect::<Box<[_]>>();

        Some(InclusionProof { path, index })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof {
    pub path: Box<[Hash]>,
    pub index: usize,
}

impl InclusionProof {
    pub fn proves(&self, root: &Hash, hash: &Hash) -> bool {
        hash.is_proved_by(root, self)
    }
}
