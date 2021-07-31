use bytes::{BufMut, Bytes, BytesMut};
use futures::channel::mpsc as futures_mpsc;
use futures::{stream, Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{btree_map::Entry, BTreeMap};
use std::marker::{PhantomData, Unpin};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{fmt, io};
use tokio::net::TcpStream;
use tokio::sync::{mpsc as tokio_mpsc, RwLock};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

/// TODO: big thing... this limites the size of the file!!! But don't set it too big
/// either (it's a trap!)
const MAX_FRAME_LENGTH: usize = 64 * 1024 * 1024; // a generous 64MB!

type SenderMap<T> = Arc<RwLock<BTreeMap<u8, tokio_mpsc::Sender<T>>>>;
type FramedDelimtedTcp = Framed<TcpStream, LengthDelimitedCodec>;

pub struct Multiplex {
    reader_sends: SenderMap<Bytes>,
    writer_send: futures_mpsc::Sender<Bytes>,
}

impl Multiplex {
    pub fn new(stream: TcpStream) -> Multiplex {
        let reader_sends = SenderMap::default();
        let (writer_send, writer_recv) = futures_mpsc::channel(1);

        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_codec();
        let framed = Framed::new(stream, codec);
        let (write, read) = framed.split();

        tokio::spawn(Self::receive(read, reader_sends.clone()));
        tokio::spawn(Self::send(write, writer_recv));

        Multiplex {
            reader_sends,
            writer_send,
        }
    }

    async fn receive(
        mut read: stream::SplitStream<FramedDelimtedTcp>,
        reader_sends: SenderMap<Bytes>,
    ) {
        while let Some(message) = read.next().await {
            log::debug!("reading {:?}", message);
            match message {
                Ok(message) if message.is_empty() => {
                    log::warn!("received empty message");
                }
                Ok(message) => {
                    let channel = message[0]; // message not empty
                    if let Some(reader_send) = reader_sends.read().await.get(&channel) {
                        log::debug!("payload received to channel {}", channel);
                        reader_send.send(message.into()).await.ok();
                        log::debug!("read");
                    } else {
                        log::warn!("tried to send a message on inactive channel {}", channel);
                    }
                }
                Err(err) => {
                    log::warn!("multiplex receiver error: {:?}", err);
                }
            }
        }
    }

    async fn send(
        mut write: stream::SplitSink<FramedDelimtedTcp, Bytes>,
        mut writer_recv: futures_mpsc::Receiver<Bytes>,
    ) {
        while let Some(message) = writer_recv.next().await {
            if message.is_empty() {
                log::warn!("tried to send empty message");
                continue;
            }

            log::debug!("writing {:?} to channel {}", message, message[0]);
            write.send(message).await.unwrap();
            log::debug!("wrote");
        }
    }

    /// Returns `None` if channel is already occupied.
    pub async fn channel<S, R>(&self, channel: u8) -> Option<ChannelTransport<S, R>> {
        match self.reader_sends.write().await.entry(channel) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vacant) => {
                let (reader_send, reader_recv) = tokio_mpsc::channel(1);
                vacant.insert(reader_send);
                Some(ChannelTransport {
                    channel,
                    reader_recv,
                    writer_send: self.writer_send.clone(),
                    _request: PhantomData,
                    _response: PhantomData,
                })
            }
        }
    }
}

pub struct ChannelTransport<S, R> {
    channel: u8,
    reader_recv: tokio_mpsc::Receiver<Bytes>,
    writer_send: futures_mpsc::Sender<Bytes>,
    _request: PhantomData<S>,
    _response: PhantomData<R>,
}

fn bincode_error_to_io(err: Box<bincode::ErrorKind>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

impl<S, R> Stream for ChannelTransport<S, R>
where
    S: Unpin,
    R: 'static + Send + Unpin + fmt::Debug + for<'a> Deserialize<'a>,
{
    type Item = Result<R, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.reader_recv).poll_recv(cx).map(|maybe| {
            maybe.map(|msg| bincode::deserialize(&msg[1..]).map_err(bincode_error_to_io))
        })
    }
}

fn send_error_to_io(err: futures_mpsc::SendError) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, err)
}

impl<S, R> Sink<S> for ChannelTransport<S, R>
where
    R: Unpin,
    S: 'static + Send + Unpin + fmt::Debug + Serialize,
{
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send)
            .poll_ready(cx)
            .map(|ready| ready.map_err(send_error_to_io))
    }

    fn start_send(mut self: Pin<&mut Self>, item: S) -> Result<(), Self::Error> {
        let channel = self.channel;

        let mut bytes = BytesMut::new();
        bytes.put_u8(channel);

        let mut writer = bytes.writer();

        bincode::serialize_into(&mut writer, &item).expect("can always serialize");
        drop(item); // maybe can save a bit of memory?

        Pin::new(&mut self.writer_send)
            .start_send(writer.into_inner().into())
            .map_err(send_error_to_io)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send)
            .poll_flush(cx)
            .map(|flush| flush.map_err(send_error_to_io))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send)
            .poll_close(cx)
            .map(|close| close.map_err(send_error_to_io))
    }
}
