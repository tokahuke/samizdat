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
            |e: Vec<_>| format!("expected {HASH_LEN} bytes; got {}", e.len()),
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
    pub fn from_bytes(thing: impl AsRef<[u8]>) -> Hash {
        Hash::new(Sha3_224::digest(thing.as_ref()))
    }

    /// Creates the hash with all the bits set to zero.
    pub const fn zero() -> Hash {
        Hash([0; HASH_LEN])
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
        Hash::from_bytes([rand.0, self.0].concat())
    }

    /// Checks whether this hash is in a Merkle tree with root `root`, given an
    /// inclusion proof. `proof.path` is ordered from leaf level upward; bit `i` of
    /// `proof.index` selects which side `self` was on at level `i` above the leaves.
    pub fn is_proved_by(&self, root: &Hash, proof: &InclusionProof) -> bool {
        let mut mask: usize = 1;
        let mut current = *self;

        for sibling in proof.path.iter() {
            // If the bit is 1, our subtree was the right child and the sibling is on the
            // left; otherwise we were the left child and the sibling is on the right.
            current = if proof.index & mask != 0 {
                sibling.rehash(&current)
            } else {
                current.rehash(sibling)
            };

            mask <<= 1;
        }

        &current == root
    }
}

/// A Merkle tree implementation that orders hashes in a list. Hashes are thus keyed by
/// their index in the list. For a map-like implementation of a Merkle tree, see the
/// [`crate::PatriciaMap`] implementation.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    /// The binary tree structure where each vector represents a level of the tree,
    /// ordered from root to leaves
    tree: Vec<Vec<Hash>>,
}

impl From<Vec<Hash>> for MerkleTree {
    /// # Panics
    ///
    /// If `vec` is empty. Use [`MerkleTree::try_from_leaves`] for fallible construction.
    fn from(vec: Vec<Hash>) -> MerkleTree {
        MerkleTree::try_from_leaves(vec).expect("MerkleTree requires at least one leaf")
    }
}

impl MerkleTree {
    /// Constructs a [`MerkleTree`] from a list of leaf hashes, returning `None` if the
    /// list is empty.
    pub fn try_from_leaves(vec: Vec<Hash>) -> Option<MerkleTree> {
        if vec.is_empty() {
            return None;
        }

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

        Some(MerkleTree {
            tree: std::iter::successors(Some(vec), |vec| {
                if vec.len() > 1 {
                    Some(iterate_level(vec))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        })
    }

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
    /// is out of range.
    ///
    /// The returned `path` is ordered from leaf level upward (leaf-side siblings first),
    /// matching the order [`Hash::is_proved_by`] expects.
    pub fn proof_for(&self, index: usize) -> Option<InclusionProof> {
        if index >= self.len() {
            return None;
        }

        let mut level_index = index;
        // Walk leaves; up, skipping the root (it has no sibling). For a tree of N leaves
        // the path length is `tree.len() - 1` (the number of edges from leaf to root).
        let depth = self.tree.len().saturating_sub(1);

        let path = self
            .tree
            .iter()
            .rev()
            .take(depth)
            .map(|level| {
                let sibling_index = level_index ^ 1;
                level_index >>= 1;

                level.get(sibling_index).copied().unwrap_or_default()
            })
            .collect::<Box<[_]>>();

        Some(InclusionProof { path, index })
    }
}

/// An inclusion proof for a given position in a Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof {
    /// The completing hashes in the Merkle tree path
    pub path: Box<[Hash]>,
    /// The claimed index position in the Merkle tree
    pub index: usize,
}

impl InclusionProof {
    /// Checks if this inclusion proof actually proves the inclusion of a given hash in a
    /// given tree, represented by its root hash.
    pub fn proves(&self, root: &Hash, hash: &Hash) -> bool {
        hash.is_proved_by(root, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Hash> {
        (0..n).map(|i| Hash::from_bytes(i.to_le_bytes())).collect()
    }

    /// Regression test for P11; the error message used to say "expected 64 bytes".
    #[test]
    fn from_str_error_reports_correct_length() {
        // 4 bytes of base64; 3 bytes of payload.
        let err = Hash::from_str("AAAA").unwrap_err().to_string();
        assert!(
            err.contains(&format!("expected {HASH_LEN} bytes")),
            "got: {err}"
        );
    }

    /// Regression test for P1; `is_proved_by` used to always return false.
    #[test]
    fn merkle_proof_round_trips() {
        let hs = leaves(8);
        let tree = MerkleTree::from(hs.clone());
        let root = tree.root();

        for (i, leaf) in hs.iter().enumerate() {
            let proof = tree.proof_for(i).expect("in range");
            assert!(
                leaf.is_proved_by(&root, &proof),
                "valid proof for index {i} was rejected"
            );
            assert!(proof.proves(&root, leaf));
        }
    }

    /// Regression test for P1+P4; proofs at odd, non-power-of-two tree sizes also work.
    #[test]
    fn merkle_proof_round_trips_odd_sizes() {
        for n in [1usize, 2, 3, 5, 7, 9, 13, 200] {
            let hs = leaves(n);
            let tree = MerkleTree::from(hs.clone());
            let root = tree.root();
            for (i, leaf) in hs.iter().enumerate() {
                let proof = tree.proof_for(i).unwrap_or_else(|| {
                    panic!("expected proof for ({n}, {i})");
                });
                assert!(
                    leaf.is_proved_by(&root, &proof),
                    "n={n} i={i} proof rejected"
                );
            }
        }
    }

    /// Regression test for P1; a proof for a different leaf must NOT verify.
    #[test]
    fn merkle_wrong_leaf_rejected() {
        let hs = leaves(8);
        let tree = MerkleTree::from(hs.clone());
        let root = tree.root();
        let proof = tree.proof_for(3).unwrap();
        // Try every other leaf; none of them should validate against index 3's proof.
        for (i, leaf) in hs.iter().enumerate() {
            if i != 3 {
                assert!(
                    !leaf.is_proved_by(&root, &proof),
                    "leaf {i} forged a proof for index 3"
                );
            }
        }
    }

    /// Regression test for P4; `proof_for(len)` used to return `Some(bogus_proof)`.
    #[test]
    fn proof_for_out_of_range_returns_none() {
        let tree = MerkleTree::from(leaves(8));
        assert!(tree.proof_for(8).is_none());
        assert!(tree.proof_for(usize::MAX).is_none());
    }

    /// Regression test for P10; `MerkleTree::from(vec![])` used to silently produce a
    /// tree whose `root()` then panicked deep inside callers.
    #[test]
    fn empty_tree_is_explicit() {
        assert!(MerkleTree::try_from_leaves(Vec::new()).is_none());
    }

    #[test]
    #[should_panic(expected = "MerkleTree requires at least one leaf")]
    fn empty_tree_from_panics_with_clear_message() {
        let _ = MerkleTree::from(Vec::<Hash>::new());
    }
}
