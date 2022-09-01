use std::cmp;

#[derive(Debug)]
pub struct HeapEntry<P, T> {
    pub priority: P,
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
