use futures::prelude::*;
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

use samizdat_common::heap_entry::HeapEntry;

use crate::CLI;

use super::Node;

#[derive(Debug)]
pub struct Room {
    participants: Arc<RwLock<BTreeMap<SocketAddr, Arc<Node>>>>,
}

impl Room {
    pub fn new() -> Room {
        let participants = Arc::default();

        Room { participants }
    }

    pub(super) async fn insert(&self, addr: SocketAddr, participant: Node) {
        self.participants
            .write()
            .await
            .insert(addr, Arc::new(participant));
    }

    pub async fn remove(&self, addr: SocketAddr) {
        log::info!("dropping client {}", addr);
        self.participants.write().await.remove(&addr);
    }

    async fn stream_peers(
        &self,
        current: SocketAddr,
    ) -> impl Stream<Item = (SocketAddr, Arc<Node>)> {
        let mut queue = BinaryHeap::new();

        for (&peer_addr, peer) in self.participants.read().await.iter() {
            let priority = if current.ip() != peer_addr.ip() {
                (peer.statistics.rand_priority() * 1e6) as i64
            } else {
                0
            };

            queue.push(HeapEntry {
                priority,
                content: (peer_addr, peer.clone()),
            });
        }

        futures::stream::iter(std::iter::from_fn(move || {
            queue.pop().map(|entry| entry.content)
        }))
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
        self.stream_peers(current)
            .into_stream()
            .flatten()
            .filter_map(move |(peer_id, peer)| map(peer_id, peer))
            .map(|outcome| async move { outcome })
            .buffer_unordered(CLI.max_resolutions_per_query)
            .take(CLI.max_candidates)
            .collect::<Vec<_>>()
    }
}
