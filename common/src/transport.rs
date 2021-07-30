use bytes::BytesMut;
use futures::{Stream, Sink, StreamExt, SinkExt};
use futures::channel::mpsc as futures_mpsc;
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as DeriveDeserialize, Serialize as DeriveSerialize};
use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{RwLock, mpsc as tokio_mpsc};
use tokio_util::codec::{Decoder, Encoder, Framed, LengthDelimitedCodec};
use std::pin::Pin;
use std::{io, fmt};
use std::task::{Context, Poll};


/// TODO: big thing... this limites the size of the file!!! But don't set it too big
/// either (it's a trap!)
const MAX_FRAME_LENGTH: usize = 64 * 1024 * 1024; // a generous 64MB!

#[derive(Debug, DeriveSerialize, DeriveDeserialize)]
struct Message<T> {
    channel: u8,
    payload: T,
}

#[derive(Debug)]
struct MessageCodec<S, R> {
    lenght_delimited_codec: LengthDelimitedCodec,
    _send: std::marker::PhantomData<S>,
    _recv: std::marker::PhantomData<R>,
}

impl<S, R> Default for MessageCodec<S, R> {
    fn default() -> MessageCodec<S, R> {
        let lenght_delimited_codec = LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_codec();
        MessageCodec {
            lenght_delimited_codec,
            _send: std::marker::PhantomData,
            _recv: std::marker::PhantomData,
        }
    }
}

impl<S: Serialize, R> Encoder<Message<S>> for MessageCodec<S, R> {
    type Error = io::Error;
    fn encode(&mut self, item: Message<S>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let encoded = bincode::serialize(&item).expect("can always serialize");
        drop(item); // maybe saves some memory? can be a good win...

        Ok(self.lenght_delimited_codec.encode(encoded.into(), dst)?)
    }
}

impl<S, R: for<'a> Deserialize<'a>> Decoder for MessageCodec<S, R> {
    type Item = Message<R>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(item) = self.lenght_delimited_codec.decode(src)? {
            let deserialized = bincode::deserialize_from(item.as_ref()).map_err(|err| {
                io::Error::new(io::ErrorKind::InvalidData, err)
            })?;
            drop(item); // maybe saves some memory? can be a good win...

            Ok(Some(deserialized))
        } else {
            Ok(None)
        }
    }
}

pub struct Multiplex<S, R> {
    reader_sends: Arc<RwLock<BTreeMap<u8, tokio_mpsc::Sender<R>>>>,
    writer_send: futures_mpsc::Sender<Message<S>>,
}

impl<S, R> Multiplex<S, R>
where
    S: 'static + Send + fmt::Debug + Serialize + for<'a> Deserialize<'a>,
    R: 'static + Send + fmt::Debug + Serialize + for<'a> Deserialize<'a>,
{
    pub fn new(stream: TcpStream) -> Multiplex<S, R> {
        // Constructor salad! Yuck!
        let reader_sends = Arc::new(RwLock::new(BTreeMap::<u8, tokio_mpsc::Sender<R>>::new()));
        let (writer_send, mut writer_recv) = futures_mpsc::channel(1);
        let framed = Framed::new(stream, MessageCodec::<S, R>::default());
        let (mut write, mut read) = framed.split();

        // TODO: manage lifetime of tasks (this may generate garbage after drop?)

        let reader_sends_task = reader_sends.clone();
        tokio::spawn(async move {
            while let Some(message) = read.next().await {
                log::debug!("got {:?}", message);
                match message {
                    Ok(message) => {
                        if let Some(reader_send) =
                            reader_sends_task.read().await.get(&message.channel)
                        {
                            log::debug!("sending payload to channel {}", message.channel);
                            reader_send.send(message.payload).await.ok();
                            log::debug!("sent");
                        } else {
                            log::warn!("tried to send a message on inactive channel {}", message.channel);
                        }
                    }
                    Err(err) => {
                        log::warn!("multiplex receiver error: {:?}", err);
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(message) = writer_recv.next().await {
                log::debug!("sending {:?}", message);
                write.send(message).await.unwrap();
            }
        });

        Multiplex {
            reader_sends,
            writer_send,
        }
    }

    /// Returns `None` if channel is already occupied.
    pub async fn channel(&self, channel: u8) -> Option<ChannelTransport<S, R>> {
        match self.reader_sends.write().await.entry(channel) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vacant) => {
                let (reader_send, reader_recv) = tokio_mpsc::channel(1);
                vacant.insert(reader_send);
                Some(ChannelTransport {
                    channel,
                    reader_recv,
                    writer_send: self.writer_send.clone(),
                })
            }
        }
    }
}

pub struct ChannelTransport<S, R> {
    channel: u8,
    reader_recv: tokio_mpsc::Receiver<R>,
    writer_send: futures_mpsc::Sender<Message<S>>,
}

impl<S, R: fmt::Debug> Stream for ChannelTransport<S, R> {
    type Item = Result<R, io::Error>;
    fn poll_next(
        mut self: Pin<&mut Self>, 
        cx: &mut Context<'_>
    ) -> Poll<Option<Self::Item>> {
        dbg!(Pin::new(&mut self.reader_recv).poll_recv(cx).map(|maybe| maybe.map(|msg| Ok(msg))))
    }
}

fn send_error_to_io(err: futures_mpsc::SendError) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, err)
}

impl<S, R> Sink<S> for ChannelTransport<S, R> {
    type Error = io::Error;

    fn poll_ready(
        mut self: Pin<&mut Self>, 
        cx: &mut Context<'_>
    ) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send).poll_ready(cx).map(|ready| ready.map_err(send_error_to_io))
    }

    fn start_send(mut self: Pin<&mut Self>, item: S) -> Result<(), Self::Error> {
        let channel = self.channel;
        Pin::new(&mut self.writer_send).start_send(Message { channel, payload: item}).map_err(send_error_to_io)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>, 
        cx: &mut Context<'_>
    ) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send).poll_flush(cx).map(|flush| flush.map_err(send_error_to_io))
    }

    fn poll_close(
        mut self: Pin<&mut Self>, 
        cx: &mut Context<'_>
    ) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer_send).poll_close(cx).map(|close| close.map_err(send_error_to_io))
    }
}
