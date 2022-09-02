//! A helper `struct` to be used in conjunction with [`std::collections::BinaryHeap`] in
//! order to make it behave like a map.

use std::cmp;

/// A helper `struct` to be used in conjunction with [`std::collections::BinaryHeap`] in
/// order to make it behave like a map.
///
/// The ordering of the [`HeapEntry`] is the same as the ordering of `P`.
#[derive(Debug)]
pub struct HeapEntry<P, T> {
    /// The ordered key used by the binary heap.
    pub priority: P,
    /// The associated content to the ordered key.
    pub content: T,
}

impl<P: Ord, T> HeapEntry<P, T> {
    fn ord(&self, other: &Self) -> cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl<P: Ord, T> PartialEq for HeapEntry<P, T> {
    fn eq(&self, other: &Self) -> bool {
        self.ord(other).is_eq()
    }
}

impl<P: Ord, T> Eq for HeapEntry<P, T> {}

impl<P: Ord, T> PartialOrd for HeapEntry<P, T> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.ord(other))
    }
}

impl<P: Ord, T> Ord for HeapEntry<P, T> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.ord(other)
    }
}
