use chashmap::CHashMap;
use futures::channel::mpsc;
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub struct Room<T> {
    next_id: AtomicUsize,
    participants: Arc<CHashMap<usize, Arc<T>>>,
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

    pub fn insert(&self, participant: T) -> Participant<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let arc = Arc::new(participant);

        // Key is guaranteed not to exist. NEVER REPEAT IDs!
        self.participants.insert(id, arc.clone());

        Participant(Arc::new(ParticipantInner {
            id,
            peers: self.participants.clone(),
            arc,
        }))
    }

    pub fn stream_peers(&self) -> mpsc::UnboundedReceiver<(usize, Arc<T>)> {
        let (sender, receiver) = mpsc::unbounded();
        let cloned = self.participants.clone();

        tokio::spawn(async move {
            cloned.retain(|&id, peer| {
                sender.unbounded_send((id, peer.clone())).ok();
                true
            });
        });

        receiver
    }
}

#[derive(Debug, Clone)]
struct ParticipantInner<T: 'static + Sync + Send> {
    pub id: usize,
    arc: Arc<T>,
    peers: Arc<CHashMap<usize, Arc<T>>>,
}

#[derive(Debug)]
pub struct Participant<T: 'static + Sync + Send>(Arc<ParticipantInner<T>>);

impl<T: 'static + Sync + Send> Clone for Participant<T> {
    fn clone(&self) -> Participant<T> {
        Participant(self.0.clone())
    }
}

impl<T: 'static + Sync + Send> Drop for ParticipantInner<T> {
    fn drop(&mut self) {
        log::debug!("droping participant");
        let peers = self.peers.clone();
        peers.remove(&self.id);
    }
}

impl<T: 'static + Sync + Send> Deref for Participant<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.arc.deref()
    }
}

// impl<T: 'static + Sync + Send> Participant<T> {
//     pub fn id(&self) -> usize {
//         self.0.id
//     }

//     pub fn for_each_peer(&self, f: impl Fn(usize, &Arc<T>)) {
//         self.0.peers.retain(|&id, participant| {
//             f(id, participant);
//             true
//         })
//     }
// }
