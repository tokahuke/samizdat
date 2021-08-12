
use crate::Hash;

fn bits<'a>(hash: &'a Hash) -> impl 'a + DoubleEndedIterator<Item = Side>  {
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

        Segment {
            slice,
            len,
        }
    }

    fn bits<'a>(&'a self) -> impl 'a + Iterator<Item = Side>  {
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

    fn hash(&self) -> Hash {
        let mut current = self.next.as_ref().expect("should be not none").hash;

        // What a shitty iterator impl, but hey, better than the 60x slower.
        // 2/3 of the time is here (see tests below)
        // HOWEVER, there is jut too much hashing zipping around here that _hashing_
        // is the slow step.
        // AND SO, the shitty iterator impl is redeemed (I tesed). 
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

    fn get(&mut self, side: Side) -> &mut Option<Child> {
        match (side, &mut self.children) {
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

impl PatriciaMap {
    pub fn new() -> PatriciaMap {
        PatriciaMap::default()
    }

    pub fn insert(&mut self, key: Hash, value: Hash) -> Option<Hash> {
        let mut bits = bits(&key);

        let mut current = &mut self.root;
        let mut next_side = bits.next().expect("bit iterator for hash is non-empty");

        let old_hash = loop {
            let maybe_child = current.get(next_side);

            // Note: very tricky to convince the borrow checker on this code.
            // *this* was the way I got to do it. DO NOT TOUCH!
            // Specific problem: double borrow on `current` or `maybe_child`.
            match maybe_child {
                Some(child) => {
                    match child.segment.follow(&mut bits) {
                        FollowStatus::FollowNode(side) => {
                            println!("follow node");
                            // Prefixes match; carry on.
                            current = child.next_mut();
                            current.is_up_to_date = false; // hash will change.
                            next_side = side;
                        }
                        FollowStatus::Split(side, split_at) => {
                            println!("split");
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
                                is_up_to_date: false, // still need to calulate hash
                                children: match side {
                                    Side::Left => [Some(leaf), Some(other_child)],
                                    Side::Right => [Some(other_child), Some(leaf)],
                                },
                            }));

                            break None;
                        }
                        FollowStatus::FoundLeaf => {
                            println!("existing leaf found");
                            // Hash was already inserted. Update and remove old.
                            let old_value = child.next_mut().hash;
                            child.next_mut().hash = value;
                            child.next_mut().is_up_to_date = false; // hash changed
                            break Some(old_value);
                        }
                    }
                }
                none_child => {
                    println!("fresh leaf found");
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

        // Reupdate (todo: make lazy, save expensive hashing)
        self.root.is_up_to_date = false;
        self.root.update();

        dbg!(&self);

        old_hash
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

        assert_eq!(segment, Segment::from_bits(bits(&segment.slice).take(segment.len as usize)));
    }

    #[test]
    fn patricia_map_insert() {
        let mut map = PatriciaMap::new();

        let some_hashes = (0..2)
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
    fn patricia_map_get() {
        unimplemented!();
    }
}
