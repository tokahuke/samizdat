use futures::prelude::*;
use quinn::{Connection, ConnectionError, IncomingUniStreams, NewConnection, RecvStream};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, MutexGuard};

use super::matcher::Matcher;

/// A multiplexer over a QUIC connection, capable of splitting its uni streams into channels.
pub struct Multiplexed {
    connection: Connection,
    senders: Arc<Mutex<BTreeMap<u32, mpsc::UnboundedSender<RecvStream>>>>,
    /// TODO: `UnboundedReceiver` needs to be changed to `Receiver` to avoid flooding.
    matcher: Matcher<u32, mpsc::UnboundedReceiver<RecvStream>>,
    is_closed: Arc<AtomicBool>,
}

async fn create_channel(
    mut guard: MutexGuard<'_, BTreeMap<u32, mpsc::UnboundedSender<RecvStream>>>,
    matcher: &Matcher<u32, mpsc::UnboundedReceiver<RecvStream>>,
    channel_id: u32,
    stream: RecvStream,
) {
    log::info!("creating new channel {:x}", channel_id);
    let (sender, recv) = mpsc::unbounded_channel();
    sender.send(stream).ok();
    guard.insert(channel_id, sender);
    drop(guard); // avoid locking while "arriving item"
    matcher.arrive(channel_id, recv).await;
}

async fn receiver_task(
    mut incoming: IncomingUniStreams,
    senders: Arc<Mutex<BTreeMap<u32, mpsc::UnboundedSender<RecvStream>>>>,
    matcher: Matcher<u32, mpsc::UnboundedReceiver<RecvStream>>,
) {
    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(mut stream) => {
                let mut id_buf = [0; 4];

                // Receive the channel id for this stream.
                if let Err(err) = stream.read_exact(&mut id_buf).await {
                    log::warn!("Error reading channel id from stream: {}", err);
                    continue;
                }

                // Decode id:
                let channel_id = u32::from_be_bytes(id_buf);
                log::debug!("stream arrived for channel {:x}", channel_id);

                // Send to the apropriate channel.
                let guard = senders.lock().await;
                if let Some(sender) = guard.get(&channel_id) {
                    // Channel may be closed... create anew!
                    if let Err(mpsc::error::SendError(not_sent)) = sender.send(stream) {
                        create_channel(guard, &matcher, channel_id, not_sent).await
                    }
                } else {
                    create_channel(guard, &matcher, channel_id, stream).await
                }
            }
            Err(ConnectionError::Reset) => {
                log::info!("Connection reset");
                break;
            }
            Err(ConnectionError::TimedOut) => {
                log::info!("Connection timed out");
                break;
            }
            Err(err) => {
                log::warn!("error receiving new stream: {}", err);
                break;
            }
        }
    }
}

impl Multiplexed {
    pub fn new(new_connection: NewConnection) -> Multiplexed {
        let senders = Arc::new(Mutex::new(
            BTreeMap::<_, mpsc::UnboundedSender<RecvStream>>::new(),
        ));
        let incoming = new_connection.uni_streams;
        let matcher = Matcher::default();
        let is_closed = Arc::new(AtomicBool::new(false));
        let set_closed = is_closed.clone();

        tokio::spawn(
            receiver_task(incoming, senders.clone(), matcher.clone())
                .map(move |_| set_closed.store(true, Ordering::Relaxed)),
        );

        Multiplexed {
            connection: new_connection.connection,
            senders,
            matcher,
            is_closed,
        }
    }

    pub async fn send(&self, channel_id: u32, payload: &[u8]) -> Result<(), crate::Error> {
        let mut stream = self.connection.open_uni().await?;
        log::debug!("stream opened for {:x}", channel_id);

        stream
            .write_all(&channel_id.to_be_bytes())
            .await
            .map_err(io::Error::from)?;
        log::debug!("channel id sent for {:x}", channel_id);

        stream.write_all(payload).await.map_err(io::Error::from)?;
        log::debug!("payload streamed for {:x}", channel_id);

        stream.finish().await.map_err(io::Error::from)?;
        log::debug!("payload sent for {:x}", channel_id);

        Ok(())
    }

    pub async fn initiate(&self, channel_id: u32) -> mpsc::UnboundedReceiver<RecvStream> {
        log::info!("initiating channel id {:x}", channel_id);
        let (sender, recv) = mpsc::unbounded_channel();
        let mut guard = self.senders.lock().await;
        guard.insert(channel_id, sender);
        recv
    }

    pub async fn expect(&self, channel_id: u32) -> Option<mpsc::UnboundedReceiver<RecvStream>> {
        log::info!("expecting channel id {:x}", channel_id);
        self.matcher.expect(channel_id).await
    }

    pub fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::Relaxed)
    }

    pub fn remote_address(&self) -> SocketAddr {
        self.connection.remote_address()
    }
}
