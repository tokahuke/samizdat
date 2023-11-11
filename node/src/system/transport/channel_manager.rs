use futures::prelude::*;
use quinn::{ReadToEndError, RecvStream};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use samizdat_common::address::{ChannelAddr, ChannelId};

use super::connection_manager::{ConnectionManager, DropMode};
use super::multiplexed::Multiplexed;

pub struct ChannelManager {
    connections: RwLock<BTreeMap<SocketAddr, Arc<Multiplexed>>>,
    connection_manager: Arc<ConnectionManager>,
}

impl ChannelManager {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> ChannelManager {
        ChannelManager {
            connections: RwLock::default(),
            connection_manager,
        }
    }

    pub async fn peers(&self) -> Vec<(SocketAddr, bool)> {
        self.connections
            .read()
            .await
            .iter()
            .map(|(addr, multiplexed)| (*addr, multiplexed.is_closed()))
            .collect()
    }

    async fn connect_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<Arc<Multiplexed>, crate::Error> {
        log::debug!("fetching connection for {}", peer_addr);

        if let Some(multiplexed) = self.connections.read().await.get(&peer_addr) {
            log::debug!("found existing connection");
            if !multiplexed.is_closed() {
                return Ok(multiplexed.clone());
            } else {
                log::debug!("existing connection already closed. Create a new one!");
            }
        }

        let mut guard = self.connections.write().await;
        log::debug!("connection write guard acquired");

        // Possible TOCTOU: check again.
        if let Some(multiplexed) = guard.remove(&peer_addr) {
            log::debug!("found existing connection on recheck");
            if !multiplexed.is_closed() {
                return Ok(multiplexed);
            } else {
                log::debug!("existing connection already closed. Create a new one!");
            }
        }

        guard.remove(&peer_addr); // force drop before new connection
        let multiplexed = Arc::new(Multiplexed::new(
            self.connection_manager
                .punch_hole_to(peer_addr, drop_mode)
                .await?,
        ));
        guard.insert(peer_addr, multiplexed.clone());

        Ok(multiplexed)
    }

    /// Waits for a given channel to be opened (i.e., the first message for it to arrive).
    pub async fn expect(
        &self,
        channel_addr: ChannelAddr,
    ) -> Result<(ChannelSender, ChannelReceiver), crate::Error> {
        let multiplexed = self
            .connect_to(channel_addr.peer_addr(), DropMode::DropOutgoing)
            .await?;
        let receiver = multiplexed
            .expect(channel_addr.channel_id())
            .await
            .ok_or_else(|| format!("channel {} was not initiated in time", channel_addr))?;

        Ok((
            ChannelSender {
                channel_id: channel_addr.channel_id(),
                multiplexed,
            },
            ChannelReceiver { receiver },
        ))
    }

    /// Initiates a given channel.
    pub async fn initiate(
        &self,
        channel_addr: ChannelAddr,
    ) -> Result<(ChannelSender, ChannelReceiver), crate::Error> {
        let multiplexed = self
            .connect_to(channel_addr.peer_addr(), DropMode::DropIncoming)
            .await?;
        let receiver = multiplexed.initiate(channel_addr.channel_id()).await;

        Ok((
            ChannelSender {
                channel_id: channel_addr.channel_id(),
                multiplexed,
            },
            ChannelReceiver { receiver },
        ))
    }
}

pub struct ChannelSender {
    channel_id: ChannelId,
    multiplexed: Arc<Multiplexed>,
}

impl ChannelSender {
    pub async fn send(&self, payload: &[u8]) -> Result<(), crate::Error> {
        self.multiplexed.send(self.channel_id, payload).await
    }

    pub fn remote_address(&self) -> ChannelAddr {
        ChannelAddr::new(self.multiplexed.remote_address(), self.channel_id)
    }
}

pub struct ChannelReceiver {
    receiver: mpsc::UnboundedReceiver<RecvStream>,
}

fn read_error_to_io(error: ReadToEndError) -> io::Error {
    match error {
        ReadToEndError::TooLong => io::Error::new(io::ErrorKind::InvalidData, "too long"),
        ReadToEndError::Read(read) => io::Error::from(read),
    }
}

impl ChannelReceiver {
    pub async fn recv(&mut self, max_len: usize) -> Result<Option<Vec<u8>>, crate::Error> {
        let outcome = if let Some(header_stream) = self.receiver.recv().await {
            header_stream
                .read_to_end(max_len)
                .await
                .map(Some)
                .map_err(read_error_to_io)
                .map_err(crate::Error::from)
        } else {
            Ok(None)
        };

        outcome
    }

    pub fn recv_many(
        mut self,
        max_len: usize,
    ) -> impl Stream<Item = Result<Vec<u8>, crate::Error>> {
        stream::poll_fn(move |ctx| self.receiver.poll_recv(ctx)).then(
            move |header_stream| async move {
                header_stream
                    .read_to_end(max_len)
                    .await
                    .map_err(read_error_to_io)
                    .map_err(crate::Error::from)
            },
        )
    }
}
