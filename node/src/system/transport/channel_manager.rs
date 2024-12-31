use futures::prelude::*;
use samizdat_common::address::{ChannelAddr, ChannelId};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::OwnedMutexGuard;
use tokio::sync::{mpsc, Mutex, RwLock};

use samizdat_common::quinn::{ReadToEndError, RecvStream};

use super::connection_manager::{connection_manager, DropMode};
use super::multiplexed::Multiplexed;

pub type PeerEntry = Arc<Mutex<Option<Arc<Multiplexed>>>>;

pub static PEER_CONNECTIONS: LazyLock<Arc<RwLock<BTreeMap<SocketAddr, PeerEntry>>>> =
    LazyLock::new(|| {
        let peers: Arc<RwLock<BTreeMap<SocketAddr, PeerEntry>>> = Arc::default();

        // Remove closed and erred connections from time to time:
        let peers_task = peers.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(10));
            loop {
                ticker.tick().await;
                peers_task.write().await.retain(|_, entry| {
                    if let Ok(guard) = entry.try_lock() {
                        if let Some(multiplexed) = guard.as_ref() {
                            !multiplexed.is_closed()
                        } else {
                            false
                        }
                    } else {
                        true
                    }
                });
            }
        });

        peers
    });

fn create_connection(
    mut locked: OwnedMutexGuard<Option<Arc<Multiplexed>>>,
    peer_addr: SocketAddr,
    drop_mode: DropMode,
) {
    tokio::spawn(async move {
        match connection_manager()
            .punch_hole_to(peer_addr, drop_mode)
            .await
        {
            Ok(conn) => {
                *locked = Some(Arc::new(Multiplexed::new(conn)));
            }
            Err(err) => {
                tracing::error!("Failed to create connection to {peer_addr}: {err}")
            }
        }
    });
}

async fn connect_to(
    peer_addr: SocketAddr,
    drop_mode: DropMode,
) -> Result<Arc<Multiplexed>, crate::Error> {
    tracing::debug!("fetching connection for {}", peer_addr);
    let mut guard = PEER_CONNECTIONS.write().await;
    tracing::debug!("connection guard acquired");

    // Get the mutex referring to the connection.
    let (multiplexed_mutex, is_new) = if let Some(mutex) = guard.get(&peer_addr).cloned() {
        (mutex, false)
    } else {
        let lock = Arc::new(Mutex::new(None));
        let locked = lock.clone().try_lock_owned().expect("resolves immediately");
        create_connection(locked, peer_addr, drop_mode);

        guard.insert(peer_addr, lock.clone());
        (lock, true)
    };

    drop(guard); // Drop outer guard before awaiting for inner guard (prevents deadlock).
    let multiplexed_guard = multiplexed_mutex.clone().lock_owned().await;

    // Return active connection, if found.
    if let Some(multiplexed) = multiplexed_guard.as_ref() {
        if !multiplexed.is_closed() {
            tracing::debug!("found existing connection");
            return Ok(multiplexed.clone());
        } else {
            tracing::debug!("existing connection already closed.");
        }
    } else {
        tracing::debug!("existing connection was unsuccessful.");
    }

    // Else, if this is a bad, but old connection, try to create a new one.
    if !is_new {
        create_connection(multiplexed_guard, peer_addr, drop_mode);
        let new_guard = multiplexed_mutex.lock_owned().await;

        // Return active connection, if found.
        if let Some(multiplexed) = new_guard.as_ref() {
            if !multiplexed.is_closed() {
                tracing::debug!("got new connection");
                return Ok(multiplexed.clone());
            } else {
                tracing::debug!("new connection already closed.");
            }
        } else {
            tracing::debug!("new connection was unsuccessful.");
        }
    }

    // If no attempt was successful, you have an error.
    Err(format!("Connection attempt to {peer_addr} was unsuccessful").into())
}

/// Waits for a given channel to be opened (i.e., the first message for it to arrive).
pub async fn expect(
    channel_addr: ChannelAddr,
) -> Result<(ChannelSender, ChannelReceiver), crate::Error> {
    let multiplexed = connect_to(channel_addr.peer_addr(), DropMode::DropOutgoing).await?;
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
    channel_addr: ChannelAddr,
) -> Result<(ChannelSender, ChannelReceiver), crate::Error> {
    let multiplexed = connect_to(channel_addr.peer_addr(), DropMode::DropIncoming).await?;
    let receiver = multiplexed.initiate(channel_addr.channel_id()).await;

    Ok((
        ChannelSender {
            channel_id: channel_addr.channel_id(),
            multiplexed,
        },
        ChannelReceiver { receiver },
    ))
}

#[derive(Clone)]
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
        if let Some(mut recv_stream) = self.receiver.recv().await {
            recv_stream
                .read_to_end(max_len)
                .await
                .map(Some)
                .map_err(read_error_to_io)
                .map_err(crate::Error::from)
        } else {
            Ok(None)
        }
    }

    pub fn recv_many(
        &mut self,
        max_len: usize,
    ) -> impl '_ + Stream<Item = Result<Vec<u8>, crate::Error>> {
        stream::poll_fn(move |ctx| self.receiver.poll_recv(ctx)).then(
            move |mut recv_stream| async move {
                recv_stream
                    .read_to_end(max_len)
                    .await
                    .map_err(read_error_to_io)
                    .map_err(crate::Error::from)
            },
        )
    }

    pub fn recv_many_owned(
        mut self,
        max_len: usize,
    ) -> impl Stream<Item = Result<Vec<u8>, crate::Error>> {
        stream::poll_fn(move |ctx| self.receiver.poll_recv(ctx)).then(
            move |mut recv_stream| async move {
                recv_stream
                    .read_to_end(max_len)
                    .await
                    .map_err(read_error_to_io)
                    .map_err(crate::Error::from)
            },
        )
    }
}
