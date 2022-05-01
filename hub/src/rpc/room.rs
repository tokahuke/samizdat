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

    pub async fn get(&self, addr: SocketAddr) -> Option<Arc<Node>> {
        self.participants.read().await.get(&addr).cloned()
    }

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
            .filter_map(move |(peer_id, peer)| {
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
            .map(|outcome| async move { outcome })
            .buffer_unordered(CLI.max_resolutions_per_query)
            .take(CLI.max_candidates)
    }

    pub async fn raw_participants<'a>(
        &'a self,
    ) -> RwLockReadGuard<'a, BTreeMap<SocketAddr, Arc<Node>>> {
        self.participants.read().await
    }
}
