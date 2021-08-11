use crate::Hash;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left = 1,
    Right = 0,
}


enum FollowStatus {
    Split(Side, u8),
    FollowNode(Side),
    FoundLeaf,
}

#[derive(Default)]
struct Segment {
    slice: Hash,
    len: u8,
}

impl Segment {
    fn bits<'a>(&'a self) -> impl 'a + DoubleEndedIterator<Item = Side> {
        self.slice.0.iter().take(self.len as usize).flat_map(|byte| {
            (0..7).map(move |i| {
                if byte & (1 << i) == 1 {
                    Side::Left
                } else {
                    Side::Right
                }
            })
        })
    }

    fn from_bits(it: impl Iterator<Item = Side>) -> Segment {
        let mut len = 0;
        let mut slice = Hash::default();

        for side in it {
            match side {
                Side::Left => slice.0[len / 8] |= 1 << (len % 8),
                Side::Right => {}
            }

            len += 1;
        }

        Segment { slice, len: len as u8 }
    }
    

    /// Recognize prefix: advance iterator until you reach the end of the prefix.
    fn follow(&self, mut it: impl Iterator<Item = Side>) -> FollowStatus {
        let mut this_it = bits(&self.slice).take(self.len as usize);
        let mut position = 0;

        loop {
            match (this_it.next(), it.next()) {
                (Some(Side::Left), Some(Side::Left)) | (Some(Side::Right), Some(Side::Right)) => {},
                (Some(Side::Right), Some(Side::Left)) => return FollowStatus::Split(Side::Left, position),
                (Some(Side::Left), Some(Side::Right)) => return FollowStatus::Split(Side::Right, position),
                (None, Some(side)) => return FollowStatus::FollowNode(side),
                (Some(_), None) => {
                    panic!("iterator should be bigger than segment")
                },
                (None, None) => return FollowStatus::FoundLeaf,
            }

            position += 1
        }
    }

    fn split(&self, other: Segment) -> Option<(Segment, Segment, Segment, Side)> {
        let mut prefix_len = 0;
        let mut this_bits = self.bits();
        let mut other_bits = other.bits();

        let mut remaining = None;
        let mut side_of_other = Side::Left;

        for (this_side, other_side) in (&mut this_bits).zip(&mut other_bits) {
            match (this_side, other_side) {
                (Side::Left, Side::Left) | (Side::Right, Side::Right) => {
                    prefix_len += 1;
                },
                (Side::Left, Side::Right) => {
                    remaining = Some((this_bits, other_bits));
                    side_of_other = Side::Right;
                    break;
                },
                (Side::Right, Side::Left) => {
                    remaining = Some((other_bits, this_bits));
                    side_of_other = Side::Left;
                    break;
                },
            }
        }

        if let Some((left, right)) = remaining {
            let new = Segment {
                slice: self.slice,
                len: prefix_len,
            };
    
            let left = Segment::from_bits(left);
            let right = Segment::from_bits(right);
    
            Some((new, left, right, side_of_other))
        } else {
            None
        }
    }
}

#[derive(Default)]
struct Node {
    children: [Option<Child>; 2],
    hash: Hash,
}

#[derive(Default)]
struct Child {
    segment: Segment,
    next: Box<Node>,
}

impl Child {
    // fn update(&mut self) {
    //     self.next.update();
    // }

    fn hash(&self) -> Hash {
        let mut current = self.next.hash;

        for side in self.segment.bits().rev() {
            match side {
                Side::Left => {
                    current = current.rehash(&Hash::default())
                },
                Side::Right => {
                    current = Hash::default().rehash(&current)
                },
            }
        }

        current
    }

    // fn split(mut self, new: Segment) -> (Child, Option<Side>) {
    //     if let Some((new, left, right, side_of_new)) = self.segment.split(new) {
    //         let new_node = Box::new(Node::new());
    //         let old_node = self.next;

    //         let (left_node, right_node) = match side_of_new {
    //             Side::Left => (new_node, old_node),
    //             Side::Right => (old_node, new_node),
    //         };

    //         (Child {
    //             segment: new,
    //             next: Box::new(Node {
    //                 children: Some([
    //                     Child { segment: left, next: left_node},
    //                     Child { segment: right, next: right_node},
    //                 ]),
    //                 hash: Hash::default(),
    //             }),
    //         }, Some(side_of_new)
    //         )
    //     } else {
    //         (self, None)
    //     }
    // }
}

impl Node {
    fn new() -> Node {
        Node::default()
    }

    // fn update(&mut self) {
    //     if let Some([left, right]) = self.children.as_mut() {
    //         left.update();
    //         right.update();

    //         self.hash = left.hash().rehash(&right.hash());
    //     }
    // }

    fn get(&mut self, side: Side) -> Option<&mut Child> {
        match (side, &mut self.children) {
            (Side::Left, [left, _]) => left.as_mut(),
            (Side::Right, [_, right]) => right.as_mut()
        }
    }

    fn insert(&mut self, child: Child, side: Side) {
        match (side, &mut self.children) {
            (Side::Left, [left, _]) => *left = Some(child),
            (Side::Right, [_, right]) => *right = Some(child),
        }
    }
}

fn bits<'a>(hash: &'a Hash) -> impl 'a + DoubleEndedIterator<Item = Side> {
    hash.0.iter().flat_map(|byte| {
        (0..7).map(move |i| {
            if byte & (1 << i) == 1 {
                Side::Left
            } else {
                Side::Right
            }
        })
    })
}

struct PatriciaMap {
    root: Node,
}

impl PatriciaMap {
    fn insert(&mut self, key: Hash, value: Hash) {
        let mut bits = bits(&key);

        let mut current = &mut self.root;
        let mut next_side = bits.next().unwrap();

        loop {
            if let Some(child) = current.get(next_side) {
                match child.segment.follow(&mut bits) {
                    FollowStatus::FollowNode(side) => {
                        current = &mut child.next;
                        next_side = side;
                    }
                    FollowStatus::Split(side, split_at) => {

                        break
                    }
                    FollowStatus::FoundLeaf => {

                        break
                    }
                }
            } else {
                let child = Child {
                    segment: Segment::from_bits(bits),
                    next: Box::new(Node {
                        children: [None, None],
                        hash: value,
                    }),
                };

                current.insert(child, next_side);

                break
            }
        }
    }
}
