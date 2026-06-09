//! Priority-plus-payload pair that lets [`BinaryHeap`] behave like a map: the heap
//! orders by `P` while carrying a `T` along for the ride.

use std::cmp;
#[cfg(doc)]
use std::collections::BinaryHeap;

/// Priority-plus-payload pair for use with [`BinaryHeap`]. Ordered by `P`; `T` rides
/// along.
#[derive(Debug)]
pub struct HeapEntry<P, T> {
    /// The ordered key used by the binary heap.
    pub priority: P,
    /// The associated content to the ordered key.
    pub content: T,
}

impl<P: Ord, T> HeapEntry<P, T> {
    /// Compares two entries by priority. Backs the `Ord` / `PartialOrd` impls.
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
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl<P: Ord, T> Ord for HeapEntry<P, T> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.ord(other)
    }
}
