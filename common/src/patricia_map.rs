//! This impl is not cache-conscious at all. However, the bottleneck here is
//! hashing (time for sha3 > cache miss). I have timed with hashing disabled
//! and it is some times faster. Therefore, by Amdahl s Law, this impl is ok.

use crate::Hash;
use serde_derive::{Deserialize, Serialize};
use std::iter::FromIterator;

/// Returns an iterator over the bits of the supplied hash.
fn bits(hash: &'_ Hash) -> impl '_ + DoubleEndedIterator<Item = Side> {
    hash.0.iter().flat_map(|byte| {
        (0..8).map(move |i| {
            if byte & (1 << i) != 0 {
                Side::Left
            } else {
                Side::Right
            }
        })
    })
}

/// The side of the binary tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left = 1,
    Right = 0,
}

impl Side {
    /// Inverts the given side.
    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

enum FollowStatus {
    Split(Side, u8),
    FollowNode(Side),
    FoundLeaf,
}

/// A segment is an edge in the Patricia tree graph. It contains a slice of binary data.
#[derive(Debug, Default, PartialEq, Eq)]
struct Segment {
    /// The slice of binary data. Because the key of the Patricia tree is a [`Hash`], we
    /// can use a [`Hash`] to contain the segment of the key here.
    slice: Hash,
    /// The length of the segment.
    len: u8,
}

impl Segment {
    /// Creates a segment from bits, encoded as binary tree sides.
    fn from_bits(it: impl Iterator<Item = Side>) -> Segment {
        let mut len = 0;
        let mut slice = Hash::default();

        for side in it {
            match side {
                Side::Left => slice.0[len as usize / 8] |= 1 << (len % 8),
                Side::Right => {}
            }

            len += 1;
        }

        Segment { slice, len }
    }

    /// Returns an iterator over the bits in this segment, represented as binary tree
    /// sides.
    fn bits(&'_ self) -> impl '_ + Iterator<Item = Side> {
        bits(&self.slice).take(self.len as usize)
    }

    /// Recognize prefix: advance iterator until you reach the end of the prefix.
    fn follow(&self, mut it: impl Iterator<Item = Side>) -> FollowStatus {
        let mut this_it = self.bits();
        let mut position = 0;

        loop {
            match (this_it.next(), it.next()) {
                (Some(this_side), Some(other_side)) if this_side == other_side => {}
                (Some(_this_side), Some(other_side)) => {
                    return FollowStatus::Split(other_side, position)
                }
                (None, Some(other_side)) => return FollowStatus::FollowNode(other_side),
                (Some(_), None) => {
                    panic!("iterator should be bigger than segment")
                }
                (None, None) => return FollowStatus::FoundLeaf,
            }

            position += 1
        }
    }

    /// Split a segment into two segments at the requested position.
    fn split_at(&self, split_at: u8) -> (Segment, Segment) {
        let prefix = Segment::from_bits(self.bits().take(split_at as usize));
        let suffix = Segment::from_bits(self.bits().skip((split_at + 1) as usize));

        (prefix, suffix)
    }
}

/// A node in the Patricia tree.
#[derive(Debug, Default)]
struct Node {
    /// The has of the current node.
    hash: Hash,
    /// Wheter this node is up to date. This is used during tree updates.
    is_up_to_date: bool,
    /// The children of this node. If the child does not exist, then it is
    /// represented by `None`.
    children: [Option<Child>; 2],
}

/// A child of a node in the Patricia tree.
#[derive(Debug, Default)]
struct Child {
    /// The segment that leads to the next node.
    segment: Segment,
    /// The node that is next in line.
    next: Option<Box<Node>>,
}

impl Child {
    /// A mutable reference to the next node.
    fn next_mut(&mut self) -> &mut Node {
        self.next.as_mut().expect("should be not none")
    }

    /// A reference to the next node.
    fn next(&self) -> &Node {
        self.next.as_ref().expect("should be not none")
    }

    /// The hash that is seen by the node that has this child.
    fn hash(&self) -> Hash {
        let mut current = self.next.as_ref().expect("should be not none").hash;

        // What a shitty iterator impl, but hey, better than the 60x slower.
        // 2/3 of the time is here (see tests below)
        // HOWEVER, there is jut too much hashing zipping around here that _hashing_
        // is the slow step.
        // AND SO, the shitty iterator impl is redeemed (I tested).
        for side in self.segment.bits().collect::<Vec<_>>().into_iter().rev() {
            match side {
                Side::Left => current = current.rehash(&Hash::default()),
                Side::Right => current = Hash::default().rehash(&current),
            }
        }

        current
    }

    /// Updates the hash value of the next node, returning the updated hash.
    fn update(&mut self) -> Hash {
        self.next_mut().update();
        self.hash()
    }
}

impl Node {
    /// Creates a leaf node with a given hash.
    fn leaf(hash: Hash) -> Node {
        Node {
            children: [None, None],
            hash,
            is_up_to_date: false,
        }
    }

    /// Gets a mutable reference to a side of the node.
    fn get_mut(&mut self, side: Side) -> &mut Option<Child> {
        match (side, &mut self.children) {
            (Side::Left, [left, _]) => left,
            (Side::Right, [_, right]) => right,
        }
    }

    /// Gets a reference to a side of the node.
    fn get(&self, side: Side) -> &Option<Child> {
        match (side, &self.children) {
            (Side::Left, [left, _]) => left,
            (Side::Right, [_, right]) => right,
        }
    }

    /// Recursively updates this node if it is not already up to date.
    fn update(&mut self) {
        // Nothing to do; avoid unnecessary recursion.
        if self.is_up_to_date {
            return;
        }

        let [left, right] = &mut self.children;

        // Is a leaf. Hash is real hash. Do *not* update.
        if left.is_none() && right.is_none() {
            self.is_up_to_date = true;
            return;
        }

        let update = |maybe_child: &mut Option<Child>| {
            maybe_child.as_mut().map(Child::update).unwrap_or_default()
        };

        self.hash = update(left).rehash(&update(right));
        self.is_up_to_date = true;
    }
}

/// A Patricia tree implementation.
///
/// A Patricia tree works just like a Merkle tree. However, while the items in the Merkle
/// tree were indexed by a [`usize`], the items in the Patricia tree are indexed by a
/// [`struct@Hash`]. Because a [`struct@Hash`] is much longer than a [`usize`], the size
/// of a Merkle tree would be too big for a reasonable implementation. The Patricia tree,
/// however, takes advantage of the sparseness inherent to a Merkle tree with big key,
/// compressing the many unoccupied nodes into "segments" and storing them in the _edges_
/// of the tree. This helps to reduce the tree to a manageable size.
///
/// For more information in Patricia trees, see
/// [Wikipedia](https://en.wikipedia.org/wiki/Radix_tree).
#[derive(Debug, Default)]
pub struct PatriciaMap {
    /// The root node of the tree.
    root: Node,
}

impl FromIterator<(Hash, Hash)> for PatriciaMap {
    fn from_iter<I: IntoIterator<Item = (Hash, Hash)>>(it: I) -> Self {
        let mut map = PatriciaMap::new();

        for (key, value) in it {
            map.insert(key, value);
        }

        map
    }
}

impl<'a> IntoIterator for &'a PatriciaMap {
    type Item = (Hash, Hash);
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl PatriciaMap {
    /// Creates an empty Patricia tree.
    pub fn new() -> PatriciaMap {
        PatriciaMap::default()
    }

    /// Returns the root node of this tree.
    pub fn root(&self) -> &Hash {
        &self.root.hash
    }

    /// Inserts a new entry into the Patricia tree, returning the old value associated to
    /// the key, if it already existed in the tree.
    pub fn insert(&mut self, key: Hash, value: Hash) -> Option<Hash> {
        let mut bits = bits(&key);

        let mut current = &mut self.root;
        let mut next_side = bits.next().expect("bit iterator for hash is non-empty");

        let old_hash = loop {
            let maybe_child = current.get_mut(next_side);

            // Note: very tricky to convince the borrow checker on this code.
            // *this* was the way I got to do it. DO NOT TOUCH!
            // Specific problem: double borrow on `current` or `maybe_child`.
            match maybe_child {
                Some(child) => {
                    match child.segment.follow(&mut bits) {
                        FollowStatus::FollowNode(side) => {
                            tracing::trace!("follow node");
                            // Prefixes match; carry on.
                            current = child.next_mut();
                            current.is_up_to_date = false; // hash will change.
                            next_side = side;
                        }
                        FollowStatus::Split(side, split_at) => {
                            tracing::trace!("split");
                            // Split
                            let (prefix, suffix) = child.segment.split_at(split_at);

                            // Create leaf:
                            let leaf = Child {
                                segment: Segment::from_bits(bits),
                                next: Some(Box::new(Node::leaf(value))),
                            };

                            // Make substitutions.
                            let other_child = Child {
                                segment: suffix,
                                next: child.next.take(),
                            };
                            child.segment = prefix;
                            child.next = Some(Box::new(Node {
                                hash: Hash::default(),
                                is_up_to_date: false, // still need to calculate hash
                                children: match side {
                                    Side::Left => [Some(leaf), Some(other_child)],
                                    Side::Right => [Some(other_child), Some(leaf)],
                                },
                            }));

                            break None;
                        }
                        FollowStatus::FoundLeaf => {
                            tracing::trace!("existing leaf found");
                            // Hash was already inserted. Update and remove old.
                            let old_value = child.next_mut().hash;
                            child.next_mut().hash = value;
                            child.next_mut().is_up_to_date = false; // hash changed
                            break Some(old_value);
                        }
                    }
                }
                none_child => {
                    tracing::trace!("fresh leaf found");
                    // Found unexplored branch.
                    let child = Child {
                        // Segment will be what's left from iterator.
                        segment: Segment::from_bits(bits),
                        next: Some(Box::new(Node::leaf(value))),
                    };

                    *none_child = Some(child);

                    break None;
                }
            }
        };

        // Re-update (todo: make lazy, save expensive hashing)
        self.root.is_up_to_date = false;
        self.root.update();

        old_hash
    }

    /// Creates the inclusion proof fot a given key in the tree. This function returns
    /// [`None`] if the key is not included in the tree.
    pub fn proof_for(&self, key: Hash) -> Option<PatriciaProof> {
        let mut bits = bits(&key);

        let mut current = &self.root;
        let mut next_side = bits.next().expect("bit iterator for hash is non-empty");
        let mut path = Vec::new();

        let found_hash = loop {
            let maybe_child = current.get(next_side);

            if let Some(child) = maybe_child {
                let other_hash = current
                    .get(next_side.other())
                    .as_ref()
                    .map(Child::hash)
                    .unwrap_or_default();
                path.push((other_hash, child.segment.len));

                match child.segment.follow(&mut bits) {
                    FollowStatus::FollowNode(side) => {
                        tracing::trace!("follow node");
                        // Prefixes match; carry on.

                        // Find the hash for the other side and push to path.

                        current = child.next();
                        next_side = side;
                    }
                    FollowStatus::Split(_, _) => {
                        tracing::trace!("split");
                        break None;
                    }
                    FollowStatus::FoundLeaf => {
                        tracing::trace!("existing leaf found");
                        // Hash was already inserted.
                        break Some(child.next().hash);
                    }
                }
            } else {
                tracing::trace!("fresh leaf found");
                break None;
            }
        };

        found_hash.map(|hash| PatriciaProof {
            claimed_key: key,
            claimed_value: hash,
            path,
        })
    }

    /// An iterator over the entries in this tree.
    pub fn iter(&self) -> Iter {
        Iter {
            stack: vec![&self.root],
            choice_stack: vec![],
            is_backtracking: false,
        }
    }
}

/// An inclusion proof for a given entry in a given tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatriciaProof {
    claimed_key: Hash,
    claimed_value: Hash,
    /// From root to leaf.
    path: Vec<(Hash, u8)>,
}

impl PatriciaProof {
    /// The value claimed to be in the tree.
    pub fn claimed_value(&self) -> &Hash {
        &self.claimed_value
    }

    /// The key claimed to be in tree.
    pub fn claimed_key(&self) -> &Hash {
        &self.claimed_key
    }

    /// Checks wheter the inclusion proof is valid for a Patricia tree with a given root
    /// hash.
    pub fn is_in(&self, root: &Hash) -> bool {
        // Check if proof is the right length:
        // (`usize` mitigates overflow shenanigans).
        let bit_length = self.path.len()
            + self
                .path
                .iter()
                .map(|(_, len)| *len as usize)
                .sum::<usize>();

        if bit_length != 224 {
            tracing::warn!(
                "proof is the wrong bit length: expected {}, got {}",
                244,
                bit_length
            );
            return false;
        }

        let mut bits = bits(&self.claimed_key).rev();
        let mut current_hash = self.claimed_value;

        for (hash, segment_len) in self.path.iter().rev() {
            for _ in 0..*segment_len {
                current_hash = match bits.next().expect("length checked") {
                    Side::Left => current_hash.rehash(&Hash::default()),
                    Side::Right => Hash::default().rehash(&current_hash),
                };
            }

            current_hash = match bits.next().expect("length checked") {
                Side::Left => current_hash.rehash(hash),
                Side::Right => hash.rehash(&current_hash),
            };
        }

        &current_hash == root
    }
}

/// An iterator over the entries in a Patricia tree.
pub struct Iter<'a> {
    stack: Vec<&'a Node>,
    choice_stack: Vec<Side>,
    is_backtracking: bool,
}

impl Iter<'_> {
    /// This function is used to walk the iterator over tree. It tries to find a sibling
    /// for the current node and, if none is found, backtracks to the previous level.
    fn sibling_or_backtrack(&mut self) {
        let _top = self.stack.pop();

        match self.choice_stack.pop() {
            Some(Side::Left) => {
                if let Some(parent) = self.stack.last() {
                    if let Some(sibling) = parent.get(Side::Right) {
                        self.stack.push(sibling.next());
                        self.choice_stack.push(Side::Right);
                        self.is_backtracking = false;
                    } else {
                        self.is_backtracking = true;
                    }
                }
            }
            Some(Side::Right) => {
                self.is_backtracking = true;
            }
            None => {}
        }
    }
}

impl Iterator for Iter<'_> {
    type Item = (Hash, Hash);

    fn next(&mut self) -> Option<(Hash, Hash)> {
        // This search loop is mounted in such a way that the values on the
        // stack represent the *exact* path from root to leaf. This path can
        // be used to extract the key.
        while let Some(current) = self.stack.last() {
            if self.is_backtracking {
                self.sibling_or_backtrack();
                continue;
            }

            // Aggressively push down:
            if let Some(left) = current.get(Side::Left) {
                self.stack.push(left.next());
                self.choice_stack.push(Side::Left);
            } else if let Some(right) = current.get(Side::Right) {
                self.stack.push(right.next());
                self.choice_stack.push(Side::Right);
            } else {
                // Found leaf. Use the stack to recover the key.
                let bits = self
                    .stack
                    .iter()
                    .zip(&self.choice_stack)
                    .flat_map(|(node, choice)| {
                        Some(*choice).into_iter().chain(
                            node.get(*choice)
                                .as_ref()
                                .expect("choice exists")
                                .segment
                                .bits(),
                        )
                    });
                let rebuilt_segment = Segment::from_bits(bits);
                let key = rebuilt_segment.slice;
                assert_eq!(rebuilt_segment.len, 224);
                let value = current.hash;

                // set to backtrack.
                self.is_backtracking = true;

                return Some((key, value));
            }
        }

        None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn from_to_bits() {
        let hash = Hash::rand();
        let segment = Segment {
            slice: hash,
            len: hash.0.len() as u8 * 8,
        };

        assert_eq!(
            segment,
            Segment::from_bits(bits(&segment.slice).take(segment.len as usize))
        );
    }

    #[test]
    fn patricia_map_insert() {
        let mut map = PatriciaMap::new();

        let some_hashes = (0..200)
            .map(|_| (Hash::rand(), Hash::rand()))
            .collect::<Vec<_>>();

        println!("inserting");

        for (key, value) in &some_hashes {
            let inserted = map.insert(*key, *value);
            assert_eq!(
                inserted, None,
                "inserting inexistent ({}, {}), got {:?}",
                key, value, inserted
            );
        }

        println!("reinserting");

        for (key, value) in &some_hashes {
            let inserted = map.insert(*key, *value);
            assert_eq!(
                inserted,
                Some(*value),
                "reinserting existent ({}, {}), got {:?}",
                key,
                value,
                inserted
            );
        }

        println!("reinserting");

        for (key, value) in &some_hashes {
            let inserted = map.insert(*key, *value);
            assert_eq!(
                inserted,
                Some(*value),
                "reinserting existent ({}, {}), got {:?}",
                key,
                value,
                inserted
            );
        }
    }

    #[test]
    fn patricia_proof() {
        let mut map = PatriciaMap::new();

        let some_hashes = (0..200)
            .map(|_| (Hash::rand(), Hash::rand()))
            .collect::<Vec<_>>();

        println!("inserting");

        for (key, value) in &some_hashes {
            map.insert(*key, *value);
        }

        println!("generating proofs");

        let proofs = some_hashes
            .iter()
            .map(|(key, _)| map.proof_for(*key).unwrap())
            .collect::<Vec<_>>();

        println!("checking proofs");

        for proof in proofs {
            assert!(dbg!(proof).is_in(map.root()), "valid proof rejected");
        }
    }

    #[test]
    fn patricia_proof_invalid() {
        let mut map = PatriciaMap::new();
        let mut other_map = PatriciaMap::new();

        let some_hashes = (0..200)
            .map(|_| (Hash::rand(), Hash::rand()))
            .collect::<Vec<_>>();

        let some_other_hashes = (0..200)
            .map(|_| (Hash::rand(), Hash::rand()))
            .collect::<Vec<_>>();

        println!("inserting");

        for (key, value) in &some_hashes {
            map.insert(*key, *value);
        }

        for (key, value) in &some_other_hashes {
            other_map.insert(*key, *value);
        }

        println!("generating proofs");

        let proofs = some_hashes
            .iter()
            .map(|(key, _)| map.proof_for(*key).unwrap())
            .collect::<Vec<_>>();

        let other_proofs = some_hashes
            .iter()
            .map(|(key, _)| map.proof_for(*key).unwrap())
            .collect::<Vec<_>>();

        println!("checking proofs");

        for proof in proofs {
            assert!(dbg!(proof).is_in(map.root()), "valid proof rejected");
        }

        println!("checking proofs");

        for proof in other_proofs {
            assert!(dbg!(proof).is_in(map.root()), "valid proof rejected");
        }
    }

    #[test]
    fn patricia_iter() {
        let some_hashes = (0..200)
            .map(|_| (Hash::rand(), Hash::rand()))
            .collect::<Vec<_>>();

        let map = some_hashes.iter().copied().collect::<PatriciaMap>();

        let mut from_map = map.iter().collect::<Vec<_>>();
        from_map.sort();
        dbg!(&from_map);

        let mut from_vec = some_hashes.iter().cloned().collect::<Vec<_>>();
        from_vec.sort();
        dbg!(&from_vec);

        for ((real_key, real_value), (key, value)) in from_map.iter().zip(&from_vec) {
            dbg!((&real_key, &key));
            assert_eq!(real_key, key, "key mismatch");
            assert_eq!(real_value, value, "value mismatch");
        }
    }
}
