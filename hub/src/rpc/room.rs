//! Implements a pool of Nodes where any two nodes may be connected to each other by this
//! Hub.

use futures::prelude::*;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};

use crate::CLI;

use super::node_sampler;
use super::node_sampler::PrioritySampler;
use super::Node;

/// Implements a pool of Nodes where any two nodes may be connected to each other by this
/// Hub.
#[derive(Debug)]
pub struct Room {
    /// A list of all the [`Node`]s in this room.
    participants: Arc<RwLock<BTreeMap<SocketAddr, Arc<Node>>>>,
}

impl Room {
    /// Creates a new, empty room.
    pub fn new() -> Room {
        let participants = Arc::default();
        Room { participants }
    }

    /// Inserts a new node into the room.
    pub(super) async fn insert(&self, addr: SocketAddr, participant: Node) {
        self.participants
            .write()
            .await
            .insert(addr, Arc::new(participant));
    }

    /// Removes a node from the room, as represented by its socket address.
    pub async fn remove(&self, addr: SocketAddr) {
        log::info!("dropping client {}", addr);
        self.participants.write().await.remove(&addr);
    }

    /// Gets a reference to a node in the room, as represented by its socket address. This
    /// returns [`None`] in case no node is found with that socket address.
    pub async fn get(&self, addr: SocketAddr) -> Option<Arc<Node>> {
        self.participants.read().await.get(&addr).cloned()
    }

    /// Lists the peers in a random (but clever) order, as defined by a priority sampler
    /// (each priority sampler looks to a different metric in the node). The current node
    /// that has cause the invoking of this function is needed so as to avoid, e.g., the
    /// hub sending the query to the same node that has sent that query, initiating a loop.
    pub async fn stream_peers<'a>(
        &'a self,
        sampler: impl 'a + PrioritySampler,
        current: SocketAddr,
    ) -> impl 'a + Stream<Item = (SocketAddr, Arc<Node>)> {
        let peers = self.participants.read().await;
        let sampler = node_sampler::sample(sampler, &peers).filter(move |(_, peer)| {
            let peer_ip = peer.addr.ip();
            let current_ip = current.ip();

            // Do not query yourself (unless loopback)! IPv4 with IPv4; IPv6 with IPv6!
            let include = (peer_ip != current_ip
                || current_ip.is_loopback() && peer.addr.port() != current.port())
                && peer_ip.is_ipv6() == current_ip.is_ipv6();

            if include {
                log::debug!("Streaming {peer_ip} for client {current_ip}");
            } else {
                log::debug!("Filtering out {peer_ip} from client {current_ip}");
            }

            include
        });

        futures::stream::iter(sampler)
    }

    /// Runs a function on each node according to a random (but clever) order, as defined
    /// by a priority sampler (each priority sampler looks to a different metric in the
    /// node). This calls [`Room::stream_peers`] and does a `filter_map` on top of the
    /// returned stream.
    pub fn with_peers<'a, F, FFut, U>(
        &'a self,
        sampler: impl 'a + PrioritySampler,
        current: SocketAddr,
        map: F,
    ) -> impl 'a + Stream<Item = U>
    where
        F: 'a + Fn(SocketAddr, Arc<Node>) -> FFut,
        FFut: 'a + Future<Output = Option<U>>,
        U: 'a,
    {
        self.stream_peers(sampler, current)
            .into_stream()
            .flatten()
            .map(move |(peer_id, peer)| {
                let fut_filter_map = map(peer_id, peer); // Cannot move out of FnMut
                async move {
                    let filter_map = fut_filter_map.await;

                    if filter_map.is_some() {
                        log::debug!("Mapping in {peer_id} for client {current}")
                    } else {
                        log::debug!("Mapping out {peer_id} from client {current}");
                    }

                    filter_map
                }
            })
            .buffer_unordered(CLI.max_resolutions_per_query)
            .filter_map(|outcome| async move { outcome })
            .take(CLI.max_candidates)
    }

    /// Gets a read handle to the underlying map backing this room.
    pub async fn raw_participants<'a>(
        &'a self,
    ) -> RwLockReadGuard<'a, BTreeMap<SocketAddr, Arc<Node>>> {
        self.participants.read().await
    }
}
