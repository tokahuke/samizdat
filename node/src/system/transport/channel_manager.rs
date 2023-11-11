use futures::prelude::*;
use quinn::{ReadToEndError, RecvStream};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

use samizdat_common::address::{ChannelAddr, ChannelId};

use super::connection_manager::{ConnectionManager, DropMode};
use super::multiplexed::Multiplexed;

#[derive(Default)]
struct ConnectionHolder {
    connections: Arc<Mutex<BTreeMap<SocketAddr, Arc<Multiplexed>>>>,
}

impl ConnectionHolder {
    fn set_to_expire(&self, addr: SocketAddr) {
        let connections = self.connections.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            log::info!("Dropping connection with {addr} due to timeout.");
            connections.lock().await.remove(&addr);
        });
    }

    async fn get_or<F, Fut>(&self, addr: SocketAddr, f: F) -> Result<Arc<Multiplexed>, crate::Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Multiplexed, crate::Error>>,
    {
        let mut connections = self.connections.lock().await;
        let connection = if let Some(connection) = connections.get(&addr).cloned() {
            if connection.is_closed() {
                log::info!("Refreshing connection with {addr} because the old was closed.");
                let new_connection = Arc::new(f().await?);
                connections.insert(addr, new_connection.clone());
                self.set_to_expire(addr);
                new_connection
            } else {
                log::debug!("Found existing connection with {addr}.");
                connection
            }
        } else {
            log::info!("Creating new connection with {addr}.");
            let new_connection = Arc::new(f().await?);
            connections.insert(addr, new_connection.clone());
            self.set_to_expire(addr);
            new_connection
        };

        Ok(connection)
    }
}

pub struct ChannelManager {
    connection_holder: ConnectionHolder,
    connection_manager: Arc<ConnectionManager>,
}

impl ChannelManager {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> ChannelManager {
        ChannelManager {
            connection_holder: Default::default(),
            connection_manager,
        }
    }

    pub async fn peers(&self) -> Vec<(SocketAddr, bool)> {
        self.connection_holder
            .connections // TODO! Don't access `connection` outside ConnectionHolder
            .lock()
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
        self.connection_holder
            .get_or(peer_addr, || async {
                Ok(Multiplexed::new(
                    self.connection_manager
                        .punch_hole_to(peer_addr, drop_mode)
                        .await?,
                ))
            })
            .await
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
