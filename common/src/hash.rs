//! Defines the standard hash format that is used in the whole Samizdat codebase.

use serde_derive::{Deserialize, Serialize};
use sha3::{Digest, Sha3_224};
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Debug, Display};
use std::ops::Deref;
use std::str::FromStr;

/// The lenght in bytes of the hash used in Samizdat.
pub const HASH_LEN: usize = 28;

/// The standard hash format that is used in Samizdat.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hash(pub [u8; HASH_LEN]);

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
        if slice.len() != HASH_LEN {
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
        hash.0[..8].clone_from_slice(&bytes[..8]);

        hash
    }
}

impl Hash {
    /// Creates a [`struct@Hash`] object from a binary hash value, which has to be 28
    /// bytes long.
    ///
    /// # Panics
    ///
    /// If the received slice does not have the correct length of 28 bytes.
    pub fn new(x: impl AsRef<[u8]>) -> Hash {
        Hash(x.as_ref().try_into().expect("bad hash value"))
    }

    /// Hashes a given piece of binary data.
    pub fn hash(thing: impl AsRef<[u8]>) -> Hash {
        Hash::new(Sha3_224::digest(thing.as_ref()))
    }

    /// Creates a random hash value, without any associated binary information.
    ///
    /// This function uses [`getrandom`] to create a hash value. If creating a lot of
    /// hashes, consider using [`Hash::rand_with`] instead.
    pub fn rand() -> Hash {
        let mut rand = [0; HASH_LEN];
        getrandom::getrandom(&mut rand).expect("getrandom failed");

        Hash(rand)
    }

    /// Just like [`Hash::rand`], but allows for a local RNG (better throughput).
    pub fn rand_with<R: rand::Rng>(rng: &mut R) -> Hash {
        let mut rand = [0; HASH_LEN];

        for rand_i in &mut rand {
            *rand_i = rng.gen();
        }

        Hash(rand)
    }

    /// Calculates the hash of the concatenation of `self` with another supplied hash.
    /// This operation is the backbone of the Merkle tree implementations.
    pub fn rehash(&self, rand: &Hash) -> Hash {
        Hash::hash([rand.0, self.0].concat())
    }

    /// Checks whether this hash value is contained in a Merkle tree with root hash `root`,
    /// given an inclusion proof.
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

/// A Merkle tree implementation that orders hashes in a list. Hashes are thus keyed by their index in the list. For a map-like implementation of a Merkle tree, see the [`crate::PatriciaMap`] implementation.
pub struct MerkleTree {
    /// The binary tree. Each item of the vector corresponds to a level of a tree.
    tree: Vec<Vec<Hash>>,
}

impl From<Vec<Hash>> for MerkleTree {
    fn from(vec: Vec<Hash>) -> MerkleTree {
        fn iterate_level(slice: &[Hash]) -> Vec<Hash> {
            slice
                .chunks(2)
                .map(|chunk| match chunk {
                    [single] => single.rehash(&Hash::default()),
                    [left, right] => left.rehash(right),
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
    /// Returns the root hash of the tree.
    pub fn root(&self) -> Hash {
        self.tree[0][0]
    }

    /// The number of items in this Merkle tree.
    pub fn len(&self) -> usize {
        self.tree.last().expect("not empty").len()
    }

    /// Whether this Merkle tree is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the list of hashes for this Merkle tree.
    pub fn hashes(&self) -> &[Hash] {
        self.tree.last().expect("not empty")
    }

    /// Builds the inclusion proof for a given item in the tree. Returns `None` if `index`
    /// out of range.
    pub fn proof_for(&self, index: usize) -> Option<InclusionProof> {
        if index > self.len() {
            return None;
        }

        let mut level_index = index;

        let path = self
            .tree
            .iter()
            .map(|level| {
                // `n-1` if `n` is odd, `n+1` is `n` is even.
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

/// An inclusion proof for a given position in a Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof {
    /// The completing hashes in the Merkle tree.
    pub path: Box<[Hash]>,
    /// The claimed index in the Merkle tree.
    pub index: usize,
}

impl InclusionProof {
    /// Checks if this inclusion proof actually proves the inclusion of a given hash in a
    /// given tree, represented by its root hash.
    pub fn proves(&self, root: &Hash, hash: &Hash) -> bool {
        hash.is_proved_by(root, self)
    }
}
