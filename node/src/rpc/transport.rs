use futures::future::join;
use futures::prelude::*;
use quinn::{
    Connecting, Connection, Endpoint, Incoming, IncomingUniStreams, NewConnection, ReadToEndError,
    RecvStream,
};
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex, MutexGuard, RwLock};
use tokio::time::{sleep, Duration};

use samizdat_common::ChannelAddr;
use samizdat_common::{quic, BincodeOverQuic};

const MAX_TRANSFER_SIZE: usize = 2_048;

struct MatcherInner<K: 'static + Ord + Copy + Send, T: 'static + Send> {
    expecting: BTreeMap<K, oneshot::Sender<T>>,
    arrived: BTreeMap<K, T>,
}

/// TODO: Matcher needs a limit in order to prevent flooding.
struct Matcher<K: 'static + Ord + Copy + Send, T: 'static + Send>(Arc<Mutex<MatcherInner<K, T>>>);

impl<K: 'static + Ord + Copy + Send, T: 'static + Send> Default for Matcher<K, T> {
    fn default() -> Matcher<K, T> {
        Matcher(Arc::new(Mutex::new(MatcherInner {
            expecting: BTreeMap::new(),
            arrived: BTreeMap::new(),
        })))
    }
}

impl<K: 'static + Ord + Copy + Send, T: 'static + Send> Clone for Matcher<K, T> {
    fn clone(&self) -> Matcher<K, T> {
        Matcher(self.0.clone())
    }
}

impl<K: 'static + Ord + Copy + Send, T: 'static + Send> Matcher<K, T> {
    pub async fn expect(&self, addr: K) -> Option<T> {
        let mut inner = self.0.lock().await;
        if let Some(item) = inner.arrived.remove(&addr) {
            Some(item)
        } else {
            let (send, recv) = oneshot::channel();
            inner.expecting.insert(addr, send);

            let cloned = self.0.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(10_000)).await;
                cloned.lock().await.expecting.remove(&addr);
            });

            recv.await.ok()
        }
    }

    pub async fn arrive(&self, addr: K, item: T) {
        let mut inner = self.0.lock().await;
        if let Some(send) = inner.expecting.remove(&addr) {
            send.send(item).ok();
        } else {
            inner.arrived.insert(addr, item);

            let cloned = self.0.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(10_000)).await;
                cloned.lock().await.arrived.remove(&addr);
            });
        }
    }
}

enum DropMode {
    DropIncoming,
    DropOutgoing,
}

pub struct ConnectionManager {
    endpoint: Endpoint,
    matcher: Matcher<SocketAddr, Connecting>,
}

impl ConnectionManager {
    pub fn new(endpoint: Endpoint, mut incoming: Incoming) -> ConnectionManager {
        let matcher: Matcher<SocketAddr, Connecting> = Matcher::default();

        let matcher_task = matcher.clone();
        tokio::spawn(async move {
            while let Some(connecting) = incoming.next().await {
                matcher_task
                    .arrive(connecting.remote_address(), connecting)
                    .await;
            }
        });

        ConnectionManager { endpoint, matcher }
    }

    pub async fn connect(
        &self,
        remote_addr: &SocketAddr,
        server_name: &str,
    ) -> Result<NewConnection, crate::Error> {
        let new_connection = quic::connect(&self.endpoint, remote_addr, server_name).await?;
        log::info!(
            "client connected to server at {}",
            new_connection.connection.remote_address()
        );

        Ok(new_connection)
    }

    pub async fn transport<S, R>(
        &self,
        remote_addr: &SocketAddr,
        server_name: &str,
    ) -> Result<BincodeOverQuic<S, R>, crate::Error>
    where
        S: 'static + Send + serde::Serialize,
        R: 'static + Send + for<'a> serde::Deserialize<'a>,
    {
        let new_connection = self.connect(remote_addr, server_name).await?;

        Ok(BincodeOverQuic::new(
            new_connection.connection.clone(),
            new_connection.uni_streams,
            MAX_TRANSFER_SIZE,
        ))
    }

    /// TODO: very basic NAT/firewall traversal stuff that works well in IPv6,
    /// but not so much in IPv4. Is there a better solution? I am already using
    /// the hub as a STUN and not many people have the means to keep a TURN.
    async fn punch_hole_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<NewConnection, crate::Error> {
        let incoming = self
            .endpoint
            .connect(&peer_addr, "localhost")
            .expect("failed to start connecting");

        let outgoing = async move {
            if let Some(connecting) = self.matcher.expect(peer_addr).await {
                Ok(connecting.await?)
            } else {
                Err(format!("peer not expected").into()) as Result<_, crate::Error>
            }
        };

        match join(incoming, outgoing).await {
            (Err(_), Ok(outgoing)) => {
                log::info!("only outgoing succeeded");
                Ok(outgoing)
            }
            (Ok(incoming), Err(_)) => {
                log::info!("only incoming succeeded");
                Ok(incoming)
            }
            (Ok(incoming), Ok(outgoing)) => {
                log::info!("both connections succeeded");
                Ok(match drop_mode {
                    DropMode::DropIncoming => {
                        log::info!("choosing outgoing");
                        outgoing
                    }
                    DropMode::DropOutgoing => {
                        log::info!("choosing incoming");
                        incoming
                    }
                })
            }
            (Err(incoming_err), Err(outgoing_err)) => {
                log::info!("both connections failed");
                log::info!("incoming error: {}", incoming_err);
                log::info!("outgoing error: {}", outgoing_err);
                // TODO: better error message here.
                Err("failed miserably".to_owned().into())
            }
        }
    }
}

/// A multiplexer over a QUIC connection, capable of spliting its uni streams into channels.
struct Multiplexed {
    connection: Connection,
    senders: Arc<Mutex<BTreeMap<u32, mpsc::UnboundedSender<RecvStream>>>>,
    /// TODO: `UnboundedReceiver` needs to be changed to `Receiver` to avoid flooding.
    matcher: Matcher<u32, mpsc::UnboundedReceiver<RecvStream>>,
}

impl Multiplexed {
    async fn create_channel(
        mut guard: MutexGuard<'_, BTreeMap<u32, mpsc::UnboundedSender<RecvStream>>>,
        matcher: &Matcher<u32, mpsc::UnboundedReceiver<RecvStream>>,
        channel_id: u32,
        stream: RecvStream,
    ) {
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

                    // Send to the apropriate channel.
                    let guard = senders.lock().await;
                    if let Some(sender) = guard.get(&channel_id) {
                        // Channel may be closed... create anew!
                        if let Err(mpsc::error::SendError(not_sent)) = sender.send(stream) {
                            Multiplexed::create_channel(guard, &matcher, channel_id, not_sent).await
                        }
                    } else {
                        Multiplexed::create_channel(guard, &matcher, channel_id, stream).await
                    }
                }
                Err(err) => {
                    log::warn!("error receiving new stream: {}", err);
                }
            }
        }
    }

    fn new(new_connection: NewConnection) -> Multiplexed {
        let senders = Arc::new(Mutex::new(
            BTreeMap::<_, mpsc::UnboundedSender<RecvStream>>::new(),
        ));
        let incoming = new_connection.uni_streams;
        let matcher = Matcher::default();

        tokio::spawn(Multiplexed::receiver_task(
            incoming,
            senders.clone(),
            matcher.clone(),
        ));

        Multiplexed {
            connection: new_connection.connection,
            senders,
            matcher,
        }
    }

    async fn send(&self, channel_id: u32, payload: &[u8]) -> Result<(), crate::Error> {
        let mut stream = self.connection.open_uni().await?;
        log::debug!("stream opened");

        stream
            .write_all(&channel_id.to_be_bytes())
            .await
            .map_err(io::Error::from)?;
        log::debug!("channel id sent");

        stream.write_all(&payload).await.map_err(io::Error::from)?;
        log::debug!("payload streamed");

        stream.finish().await.map_err(io::Error::from)?;
        log::debug!("payload sent");

        Ok(())
    }

    async fn initiate(&self, channel_id: u32) -> mpsc::UnboundedReceiver<RecvStream> {
        let (sender, recv) = mpsc::unbounded_channel();
        let mut guard = self.senders.lock().await;
        guard.insert(channel_id, sender);
        recv
    }

    async fn expect(&self, channel_id: u32) -> Option<mpsc::UnboundedReceiver<RecvStream>> {
        self.matcher.expect(channel_id).await
    }
}

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

    async fn connect_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<Arc<Multiplexed>, crate::Error> {
        if let Some(multiplexed) = self.connections.read().await.get(&peer_addr) {
            return Ok(multiplexed.clone());
        }

        let mut guard = self.connections.write().await;

        // Possible TOCTOU: check again.
        if let Some(multiplexed) = guard.get(&peer_addr) {
            return Ok(multiplexed.clone());
        }

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

    /// Initiates a guiven channel.
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
    channel_id: u32,
    multiplexed: Arc<Multiplexed>,
}

impl ChannelSender {
    pub async fn send(&self, payload: &[u8]) -> Result<(), crate::Error> {
        self.multiplexed.send(self.channel_id, payload).await
    }

    pub fn remote_address(&self) -> ChannelAddr {
        ChannelAddr::new(
            self.multiplexed.connection.remote_address(),
            self.channel_id,
        )
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
                .map(|msg| Some(msg))
                .map_err(read_error_to_io)?
        } else {
            None
        };

        Ok(outcome)
    }

    pub fn recv_many<'a>(
        &'a mut self,
        max_len: usize,
    ) -> impl 'a + Stream<Item = Result<Vec<u8>, crate::Error>> {
        stream::poll_fn(move |ctx| self.receiver.poll_recv(ctx)).then(
            move |header_stream| async move {
                Ok(header_stream
                    .read_to_end(max_len)
                    .await
                    .map_err(read_error_to_io)?)
            },
        )
    }
}
