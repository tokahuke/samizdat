use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// TODO: failed to use CHashMap... bugs iterating... v2.2.2

#[derive(Debug)]
pub struct Room<T> {
    next_id: AtomicUsize,
    participants: Arc<RwLock<BTreeMap<usize, Arc<T>>>>,
}

impl<T: 'static + Send + Sync> Room<T> {
    pub fn new() -> Room<T> {
        let next_id = AtomicUsize::new(1_024); // just 'cause...
        let participants = Arc::default();

        Room {
            next_id,
            participants,
        }
    }

    pub async fn insert(&self, participant: T) -> Participant<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let arc = Arc::new(participant);

        // Key is guaranteed not to exist. NEVER REPEAT IDs!
        self.participants.write().await.insert(id, arc.clone());

        Participant(Arc::new(ParticipantInner {
            id,
            peers: self.participants.clone(),
            arc,
        }))
    }
}

#[derive(Debug, Clone)]
struct ParticipantInner<T: 'static + Sync + Send> {
    pub id: usize,
    arc: Arc<T>,
    peers: Arc<RwLock<BTreeMap<usize, Arc<T>>>>,
}

#[derive(Debug, Clone)]
pub struct Participant<T: 'static + Sync + Send>(Arc<ParticipantInner<T>>);

impl<T: 'static + Sync + Send> Drop for ParticipantInner<T> {
    fn drop(&mut self) {
        log::debug!("droping participant");
        let peers = self.peers.clone();
        let id = self.id;
        tokio::spawn(async move {
            peers.write().await.remove(&id);
        });
    }
}

impl<T: 'static + Sync + Send> Deref for Participant<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.arc.deref()
    }
}

impl<T: 'static + Sync + Send> Participant<T> {
    pub fn id(&self) -> usize {
        self.0.id
    }

    pub async fn for_each_peer(&self, f: impl Fn(usize, &Arc<T>)) {
        for (&id, participant) in self.0.peers.read().await.iter() {
            f(id, participant)
        }
    }
}
