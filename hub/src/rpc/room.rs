use chashmap::CHashMap;
use futures::channel::mpsc;
use futures::prelude::*;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::CLI;

use super::Node;

#[derive(Debug)]
pub struct Room {
    next_id: AtomicUsize,
    participants: Arc<CHashMap<SocketAddr, Arc<Node>>>,
}

impl Room {
    pub fn new() -> Room {
        let next_id = AtomicUsize::new(1_024); // just 'cause...
        let participants = Arc::default();

        Room {
            next_id,
            participants,
        }
    }

    pub(super) fn insert(&self, addr: SocketAddr, participant: Node) {
        // Key is guaranteed not to exist. NEVER REPEAT IDs!
        self.participants.insert(addr, Arc::new(participant));
    }

    pub fn remove(&self, addr: SocketAddr) {
        log::info!("dropping client {}", addr);
        self.participants.remove(&addr);
    }

    pub(super) fn stream_peers(&self) -> mpsc::UnboundedReceiver<(SocketAddr, Arc<Node>)> {
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

    pub(super) fn with_peers<'a, F, FFut, U>(
        &'a self,
        current: SocketAddr,
        map: F,
    ) -> impl 'a + Future<Output = Vec<U>>
    where
        F: 'a + Fn(SocketAddr, Arc<Node>) -> FFut,
        FFut: 'a + Future<Output = Option<U>>,
        U: 'a,
    {
        self.stream_peers()
            .filter_map(move |(peer_id, peer)| {
                let peer_addr = peer.addr;
                let mapped = map(peer_id, peer);
                async move {
                    if peer_addr != current {
                        mapped.await
                    } else {
                        None
                    }
                }
            })
            .map(|outcome| async move { outcome })
            .buffer_unordered(CLI.max_resolutions_per_query)
            .take(CLI.max_candidates)
            .collect::<Vec<_>>()
    }
}

// #[derive(Debug, Clone)]
// struct ParticipantInner<T: 'static + Sync + Send> {
//     pub id: usize,
//     arc: Arc<T>,
//     peers: Arc<CHashMap<usize, Arc<T>>>,
// }

// #[derive(Debug)]
// pub struct Participant<T: 'static + Sync + Send>(Arc<ParticipantInner<T>>);

// impl<T: 'static + Sync + Send> Clone for Participant<T> {
//     fn clone(&self) -> Participant<T> {
//         Participant(self.0.clone())
//     }
// }

// impl<T: 'static + Sync + Send> Drop for ParticipantInner<T> {
//     fn drop(&mut self) {
//         // log::debug!("dropping participant");
//         // let peers = self.peers.clone();
//         // peers.remove(&self.id);
//     }
// }

// impl<T: 'static + Sync + Send> Deref for Participant<T> {
//     type Target = T;
//     fn deref(&self) -> &T {
//         self.0.arc.deref()
//     }
// }

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
