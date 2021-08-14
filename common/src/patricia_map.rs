//! This impl is not cache-conscious at all. However, the bottleneck here is
//! hashing (time for sha3 > cache miss). I have timed with hashing disabled
//! and it is some times faster. Therefore, by Amdahl s Law, this impl is ok.

use crate::Hash;
use serde_derive::{Deserialize, Serialize};
use std::iter::FromIterator;

fn bits<'a>(hash: &'a Hash) -> impl 'a + DoubleEndedIterator<Item = Side> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left = 1,
    Right = 0,
}

impl Side {
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

#[derive(Debug, Default, PartialEq, Eq)]
struct Segment {
    slice: Hash,
    len: u8,
}

impl Segment {
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

    fn bits<'a>(&'a self) -> impl 'a + Iterator<Item = Side> {
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

    fn split_at(&self, split_at: u8) -> (Segment, Segment) {
        let prefix = Segment::from_bits(self.bits().take(split_at as usize));
        let suffix = Segment::from_bits(self.bits().skip((split_at + 1) as usize));

        (prefix, suffix)
    }
}

#[derive(Debug, Default)]
struct Node {
    hash: Hash,
    is_up_to_date: bool,
    children: [Option<Child>; 2],
}

#[derive(Debug, Default)]
struct Child {
    segment: Segment,
    next: Option<Box<Node>>,
}

impl Child {
    fn next_mut(&mut self) -> &mut Node {
        self.next.as_mut().expect("should be not none")
    }

    fn next(&self) -> &Node {
        self.next.as_ref().expect("should be not none")
    }

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

    fn update(&mut self) -> Hash {
        self.next_mut().update();
        self.hash()
    }
}

impl Node {
    fn leaf(hash: Hash) -> Node {
        Node {
            children: [None, None],
            hash,
            is_up_to_date: false,
        }
    }

    fn get_mut(&mut self, side: Side) -> &mut Option<Child> {
        match (side, &mut self.children) {
            (Side::Left, [left, _]) => left,
            (Side::Right, [_, right]) => right,
        }
    }

    fn get(&self, side: Side) -> &Option<Child> {
        match (side, &self.children) {
            (Side::Left, [left, _]) => left,
            (Side::Right, [_, right]) => right,
        }
    }

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

#[derive(Debug, Default)]
pub struct PatriciaMap {
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
    pub fn new() -> PatriciaMap {
        PatriciaMap::default()
    }

    pub fn root(&self) -> &Hash {
        &self.root.hash
    }

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
                            log::trace!("follow node");
                            // Prefixes match; carry on.
                            current = child.next_mut();
                            current.is_up_to_date = false; // hash will change.
                            next_side = side;
                        }
                        FollowStatus::Split(side, split_at) => {
                            log::trace!("split");
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
                            log::trace!("existing leaf found");
                            // Hash was already inserted. Update and remove old.
                            let old_value = child.next_mut().hash;
                            child.next_mut().hash = value;
                            child.next_mut().is_up_to_date = false; // hash changed
                            break Some(old_value);
                        }
                    }
                }
                none_child => {
                    log::trace!("fresh leaf found");
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
                        log::trace!("follow node");
                        // Prefixes match; carry on.

                        // Find the hash for the other side and push to path.

                        current = child.next();
                        next_side = side;
                    }
                    FollowStatus::Split(_, _) => {
                        log::trace!("split");
                        break None;
                    }
                    FollowStatus::FoundLeaf => {
                        log::trace!("existing leaf found");
                        // Hash was already inserted.
                        break Some(child.next().hash);
                    }
                }
            } else {
                log::trace!("fresh leaf found");
                break None;
            }
        };

        found_hash.map(|hash| PatriciaProof {
            claimed_key: key,
            claimed_value: hash,
            path,
        })
    }

    pub fn iter(&self) -> Iter {
        Iter {
            stack: vec![&self.root],
            choice_stack: vec![],
            is_backtracking: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatriciaProof {
    claimed_key: Hash,
    claimed_value: Hash,
    /// From root to leaf.
    path: Vec<(Hash, u8)>,
}

impl PatriciaProof {
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
            log::warn!(
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

pub struct Iter<'a> {
    stack: Vec<&'a Node>,
    choice_stack: Vec<Side>,
    is_backtracking: bool,
}

impl<'a> Iter<'a> {
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

impl<'a> Iterator for Iter<'a> {
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

        return None;
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
