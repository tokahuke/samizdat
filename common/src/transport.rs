use bytes::BytesMut;
use futures::channel::mpsc as futures_mpsc;
use futures::{stream, Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as DeriveDeserialize, Serialize as DeriveSerialize};
use std::collections::{btree_map::Entry, BTreeMap};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{fmt, io};
use tokio::net::TcpStream;
use tokio::sync::{mpsc as tokio_mpsc, RwLock};
use tokio_util::codec::{Decoder, Encoder, Framed, LengthDelimitedCodec};

/// TODO: big thing... this limites the size of the file!!! But don't set it too big
/// either (it's a trap!)
const MAX_FRAME_LENGTH: usize = 64 * 1024 * 1024; // a generous 64MB!

#[derive(Debug, DeriveSerialize, DeriveDeserialize)]
enum Either<S, R> {
    Request(S),
    Response(R),
}

#[derive(Debug, DeriveSerialize, DeriveDeserialize)]
struct Message<T> {
    channel: u8,
    payload: T,
}

#[derive(Debug)]
struct MessageCodec<T> {
    lenght_delimited_codec: LengthDelimitedCodec,
    _send: std::marker::PhantomData<T>,
}

impl<T> Default for MessageCodec<T> {
    fn default() -> MessageCodec<T> {
        let lenght_delimited_codec = LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_codec();
        MessageCodec {
            lenght_delimited_codec,
            _send: std::marker::PhantomData,
        }
    }
}

impl<T: Serialize> Encoder<Message<T>> for MessageCodec<T> {
    type Error = io::Error;
    fn encode(&mut self, item: Message<T>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let encoded = bincode::serialize(&item).expect("can always serialize");
        drop(item); // maybe saves some memory? can be a good win...

        Ok(self.lenght_delimited_codec.encode(encoded.into(), dst)?)
    }
}

impl<T: for<'a> Deserialize<'a>> Decoder for MessageCodec<T> {
    type Item = Message<T>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(item) = self.lenght_delimited_codec.decode(src)? {
            let deserialized = bincode::deserialize_from(item.as_ref())
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            drop(item); // maybe saves some memory? can be a good win...

            Ok(Some(deserialized))
        } else {
            Ok(None)
        }
    }
}

type SenderMap<T> = Arc<RwLock<BTreeMap<u8, tokio_mpsc::Sender<T>>>>;
type ChannelSender<T> = futures_mpsc::Sender<(u8, T)>;
type ChannelRecv<T> = futures_mpsc::Receiver<(u8, T)>;

pub struct Multiplex<S, R> {
    request_reader_sends: SenderMap<S>,
    response_reader_sends: SenderMap<R>,
    request_writer_send: ChannelSender<S>,
    response_writer_send: ChannelSender<R>,
}

impl<S, R> Multiplex<S, R>
where
    S: 'static + Send + fmt::Debug + Serialize + for<'a> Deserialize<'a>,
    R: 'static + Send + fmt::Debug + Serialize + for<'a> Deserialize<'a>,
{
    pub fn new(stream: TcpStream) -> Multiplex<S, R> {
        // Constructor salad! Yuck!
        let request_reader_sends = SenderMap::default();
        let response_reader_sends = SenderMap::default();
        let (request_writer_send, request_writer_recv) = futures_mpsc::channel(1);
        let (response_writer_send, response_writer_recv) = futures_mpsc::channel(1);

        let framed = Framed::new(stream, MessageCodec::<Either<S, R>>::default());
        let (write, read) = framed.split();

        // TODO: manage lifetime of tasks (this may generate garbage after drop?)
        tokio::spawn(Self::receive(
            read,
            request_reader_sends.clone(),
            response_reader_sends.clone(),
        ));
        tokio::spawn(Self::send(write, request_writer_recv, response_writer_recv));

        Multiplex {
            request_reader_sends,
            response_reader_sends,
            request_writer_send,
            response_writer_send,
        }
    }
    async fn receive(
        mut read: stream::SplitStream<Framed<TcpStream, MessageCodec<Either<S, R>>>>,
        request_reader_sends: SenderMap<S>,
        response_reader_sends: SenderMap<R>,
    ) {
        while let Some(message) = read.next().await {
            log::debug!("got {:?}", message);

            async fn route<T>(reader_sends: &SenderMap<T>, channel: u8, payload: T) {
                if let Some(reader_send) = reader_sends.read().await.get(&channel) {
                    log::debug!("sending payload to channel {}", channel);
                    reader_send.send(payload).await.ok();
                    log::debug!("sent");
                } else {
                    log::warn!("tried to send a message on inactive channel {}", channel);
                }
            }

            match message {
                Ok(message) => match message.payload {
                    Either::Request(request) => {
                        route(&request_reader_sends, message.channel, request).await
                    }
                    Either::Response(response) => {
                        route(&response_reader_sends, message.channel, response).await
                    }
                },
                Err(err) => {
                    log::warn!("multiplex receiver error: {:?}", err);
                }
            }
        }
    }

    async fn send(
        mut write: stream::SplitSink<
            Framed<TcpStream, MessageCodec<Either<S, R>>>,
            Message<Either<S, R>>,
        >,
        request_writer_recv: ChannelRecv<S>,
        response_writer_recv: ChannelRecv<R>,
    ) {
        let request_messages = request_writer_recv.map(|(channel, request)| Message {
            channel,
            payload: Either::Request(request),
        });
        let response_messages = response_writer_recv.map(|(channel, response)| Message {
            channel,
            payload: Either::Response(response),
        });

        let mut writer_recv = stream::select(request_messages, response_messages);

        while let Some(message) = writer_recv.next().await {
            log::debug!("sending {:?}", message);
            write.send(message).await.unwrap();
        }
    }

    /// Returns `None` if channel is already occupied.
    pub async fn channel(&self, channel: u8) -> Option<ChannelTransport<S, R>> {
        match self.response_reader_sends.write().await.entry(channel) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vacant) => {
                let (reader_send, reader_recv) = tokio_mpsc::channel(1);
                vacant.insert(reader_send);
                Some(ChannelTransport {
                    channel,
                    reader_recv,
                    writer_send: self.request_writer_send.clone(),
                })
            }
        }
    }

    pub async fn reverse_channel(&self, channel: u8) -> Option<ChannelTransport<R, S>> {
        match self.request_reader_sends.write().await.entry(channel) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vacant) => {
                let (reader_send, reader_recv) = tokio_mpsc::channel(1);
                vacant.insert(reader_send);
                Some(ChannelTransport {
                    channel,
                    reader_recv,
                    writer_send: self.response_writer_send.clone(),
                })
            }
        }
    }
}

pub struct ChannelTransport<S, R> {
    channel: u8,
    reader_recv: tokio_mpsc::Receiver<R>,
    writer_send: ChannelSender<S>,
}

impl<S, R: fmt::Debug> Stream for ChannelTransport<S, R> {
    type Item = Result<R, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.reader_recv)
            .poll_recv(cx)
            .map(|maybe| maybe.map(|msg| Ok(msg)))
    }
}

fn send_error_to_io(err: futures_mpsc::SendError) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, err)
}

impl<S, R> Sink<S> for ChannelTransport<S, R> {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send)
            .poll_ready(cx)
            .map(|ready| ready.map_err(send_error_to_io))
    }

    fn start_send(mut self: Pin<&mut Self>, item: S) -> Result<(), Self::Error> {
        let channel = self.channel;
        Pin::new(&mut self.writer_send)
            .start_send((channel, item))
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
