
use crate::Hash;

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

#[derive(Default)]
struct Node {
    children: Option<[Child; 2]>,
    hash: Hash,
    segment: (Vec<u8>, u8),
}

#[derive(Default)]
struct Child {
    segment: (Vec<u8>, u8),
    next: Box<Node>,
}

impl Node {
    fn new() -> Node {
        Node::default()
    }

    fn get_or_fill_children(&mut self) -> &mut [Box<Node>; 2] {
        self.children.get_or_insert_with(|| <[Box<Node>; 2]>::default())
    }

    fn update(&mut self) {
        self.hash = if let Some([left, right]) = &self.children {
            left.hash.rehash(&right.hash)
        } else {
            return;
        };
    }

    fn get(&self, side: Side) -> Option<&Node> {
        match (side, &self.children) {
            (_, None) => None,
            (Side::Left, Some([left, _])) => {
                Some(&*left)
            },
            (Side::Right, Some([_, right])) => {
                Some(&*right)
            }
        }
    }

    fn get_or_insert(&mut self, side: Side) -> &mut Node {
        match (side, self.get_or_fill_children()) {
            (Side::Left, [left, _]) => {
                left
            },
            (Side::Right, [_, right]) => {
                right
            },
        }
    }

    /// Returns the old hash if assigned.
    fn assign(&mut self, hash: Hash) -> Option<Hash> {
        assert!(self.children.is_none(), "assigning to non-leaf node");
        if self.hash != Hash::default() {
            let old = self.hash;
            self.hash = hash;
            Some(old)
        } else {
            self.hash = hash;
            None
        }
    }

    fn get_assigned(&self) -> Option<Hash> {
        assert!(self.children.is_none(), "getting assigned to non-leaf node");
        if self.hash != Hash::default() {
            Some(self.hash)
        } else {
            None
        }
    }

    /// Panics if node is not assgned.
    fn get_proof(&self, side: Side) -> Option<(Hash, &Node)> {
        match (side, &self.children) {
            (Side::Left, Some([left, right])) => {
                Some((right.hash, left))
            },
            (Side::Right, Some([left, right])) => {
                Some((left.hash, right))
            }
            (_, None) => {
                None
            }
        }
    }
}

pub struct MerkleMap {
    root: Node,
}

fn bits<'a>(slice: &'a [u8]) -> impl 'a + Iterator<Item = bool> {
    slice.iter().flat_map(|byte| (0..7).map(move |i| byte & (1 << i) == 1))
}

impl MerkleMap {
    pub fn new() -> MerkleMap {
        MerkleMap {
            root: Node::new(),
        }
    }

    pub fn insert<K>(&mut self, key: K, value: Hash) -> Option<Hash>
    where
        K: AsRef<Hash>,
    {
        fn update_tree(hash: Hash, node: &mut Node, mut bits: impl Iterator<Item = bool>) -> Option<Hash> {
            if let Some(bit) = bits.next() {
                let side = if bit { Side::Left } else { Side::Right };
                let old = update_tree(hash, node.get_or_insert(side), bits);
                node.update();
                old
            } else {
                node.assign(hash)
            }
        }
        
        update_tree(value, &mut self.root, bits(key.as_ref()))
    }

    pub fn get<K>(&self, key: K) -> Option<Hash>
    where
        K: AsRef<Hash>,
    {
        let mut node = &self.root;

        for bit in bits(key.as_ref()) {
            let side = if bit { Side::Left } else { Side::Right };
            if let Some(child) = node.get(side) {
                node = child;
            } else {
                return None;
            }
        }

        node.get_assigned()
    }

    pub fn proof_for<K>(&self, key: K) -> Option<InclusionProof>
    where
        K: AsRef<Hash>,
    {
        let mut node = &self.root;
        let mut path = Vec::new();

        for bit in bits(key.as_ref()) {
            let side = if bit { Side::Left } else { Side::Right };
            if let Some((step, child)) = node.get_proof(side) {
                path.push(step);
                node = child;
            } else {
                return None;
            }
        }

        if node.is_assigned {
            Some(InclusionProof { claimed_key: key.as_ref().to_vec(), path })
        } else {
            None
        }
    }
}

pub struct InclusionProof {
    claimed_key: Vec<u8>,
    path: Vec<(Hash, Hash)>,
}

impl InclusionProof {
    pub fn validate(&self) -> InclusionProof {
        todo!()
    }
}
