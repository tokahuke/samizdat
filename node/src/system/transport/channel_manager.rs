use futures::prelude::*;
use lazy_static::lazy_static;
use quinn::{ReadToEndError, RecvStream};
use samizdat_common::address::{ChannelAddr, ChannelId};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use super::connection_manager::{ConnectionManager, DropMode};
use super::multiplexed::Multiplexed;

lazy_static! {
    pub static ref PEER_CONNECTIONS: Mutex<BTreeMap<SocketAddr, Arc<Mutex<Option<Arc<Multiplexed>>>>>> =
        Mutex::default();
}

pub struct ChannelManager {
    connection_manager: Arc<ConnectionManager>,
}

impl ChannelManager {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> ChannelManager {
        ChannelManager { connection_manager }
    }

    async fn connect_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<Arc<Multiplexed>, crate::Error> {
        log::debug!("fetching connection for {}", peer_addr);
        let mut guard = PEER_CONNECTIONS.lock().await;
        log::debug!("connection guard acquired");

        if let Some(mutex) = guard.get(&peer_addr) {
            if let Some(multiplexed) = mutex.lock().await.as_ref() {
                if !multiplexed.is_closed() {
                    log::debug!("found existing connection");
                    return Ok(multiplexed.clone());
                } else {
                    log::debug!("existing connection already closed. Create a new one!");
                }
            } else {
                log::debug!("last connection attempt unsuccessful");
            }
        }

        let lock = Arc::new(Mutex::new(None));
        let mut locked = lock.clone().lock_owned().await;
        guard.insert(peer_addr, lock.clone());
        drop(guard);

        let connection_manager = self.connection_manager.clone();
        tokio::spawn(async move {
            match connection_manager.punch_hole_to(peer_addr, drop_mode).await {
                Ok(conn) => {
                    *locked = Some(Arc::new(Multiplexed::new(conn)));
                },
                Err(err) => {
                    log::error!("Failed to create connection to {peer_addr}: {err}")
                }
            }
        });

        let locked = lock.lock().await;
        if let Some(multiplexed) = locked.as_ref() {
            Ok(multiplexed.clone())
        } else {
            Err(format!("Connection attempt to {peer_addr} was unsuccessful").into())
        }
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
