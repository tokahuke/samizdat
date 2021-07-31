use chashmap::CHashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub struct Room<T> {
    next_id: AtomicUsize,
    participants: Arc<CHashMap<usize, Arc<T>>>,
}

impl<T> Drop for Participant<T> {
    fn drop(&mut self) {
        self.peers.remove(&self.id);
    }
}

impl<T> Room<T> {
    pub fn new() -> Room<T> {
        let next_id = AtomicUsize::new(1_024); // just 'cause...
        let participants = Arc::new(CHashMap::with_capacity(100));

        Room {
            next_id,
            participants,
        }
    }

    pub fn insert(&self, participant: T) -> Participant<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let arc = Arc::new(participant);

        // Key is guaranteed not to exist. NEVER REPEAT IDs!
        self.participants.insert_new(id, arc.clone());

        Participant {
            id,
            peers: self.participants.clone(),
            arc,
        }
    }
}

#[derive(Clone)]
pub struct Participant<T> {
    pub id: usize,
    arc: Arc<T>,
    peers: Arc<CHashMap<usize, Arc<T>>>,
}

impl<T> Deref for Participant<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.arc.deref()
    }
}

impl<T> Participant<T> {
    pub fn for_each_peer(&self, f: impl Fn(usize, &Arc<T>)) {
        self.peers.retain(|&id, participant| {
            f(id, participant);
            true
        })
    }
}
