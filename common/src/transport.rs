use bytes::{BufMut, Bytes, BytesMut};
use futures::channel::mpsc as futures_mpsc;
use futures::prelude::*;
use quinn::generic::{OpenBi, RecvStream, SendStream, ServerConfig};
use quinn::{crypto::Session, Connection, Endpoint, Incoming};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, IoSlice};
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub struct Transport {
    addr: SocketAddr,
    endpoint: Endpoint,
    incoming: Incoming,
}

impl Transport {
    pub async fn start(addr: SocketAddr) -> Result<Transport, crate::Error> {
        let mut endpoint_builder = Endpoint::builder();
        endpoint_builder.listen(ServerConfig::default());

        let (endpoint, incoming) = endpoint_builder.bind(&addr).expect("failed to bind");

        Ok(Transport {
            addr,
            endpoint,
            incoming,
        })
    }
}

pub struct QuicTransport<S: Session> {
    send: SendStream<S>,
    recv: RecvStream<S>,
}

impl<S: Session> AsyncRead for QuicTransport<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl<S: Session> AsyncWrite for QuicTransport<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.send.is_write_vectored()
    }
}

impl<S: Session> QuicTransport<S> {
    pub fn new((send, recv): (SendStream<S>, RecvStream<S>)) -> QuicTransport<S> {
        QuicTransport { send, recv }
    }
}

/// TODO: big thing... this limites the size of the file!!! But don't set it too big
/// either (it's a trap!)
const MAX_FRAME_LENGTH: usize = 64 * 1024 * 1024; // a generous 64MB!
type FramedDelimtedQuic<S> = Framed<QuicTransport<S>, LengthDelimitedCodec>;

pub struct BincodeTransport<Ss: Session, S, R> {
    read: stream::SplitStream<FramedDelimtedQuic<Ss>>,
    write: stream::SplitSink<FramedDelimtedQuic<Ss>, Bytes>,
    _request: PhantomData<S>,
    _response: PhantomData<R>,
}

impl<Ss: Session, S, R> BincodeTransport<Ss, S, R> {
    pub fn new(transport: QuicTransport<Ss>) -> BincodeTransport<Ss, S, R> {
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_codec();
        let framed = Framed::new(transport, codec);
        let (write, read) = framed.split();

        BincodeTransport {
            read,
            write,
            _request: PhantomData,
            _response: PhantomData,
        }
    }
}

fn bincode_error_to_io(err: Box<bincode::ErrorKind>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

impl<Ss: Session, S, R> Stream for BincodeTransport<Ss, S, R>
where
    S: Unpin,
    R: 'static + Send + Unpin + fmt::Debug + for<'a> Deserialize<'a>,
{
    type Item = Result<R, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        log::info!("poll next");
        Pin::new(&mut self.read)
            .poll_next(cx)
            .map(|maybe| maybe.map(|msg| bincode::deserialize(&msg?).map_err(bincode_error_to_io)))
    }
}

impl<Ss: Session, S, R> Sink<S> for BincodeTransport<Ss, S, R>
where
    R: Unpin,
    S: 'static + Send + Unpin + fmt::Debug + Serialize,
{
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        log::info!("poll ready");
        Pin::new(&mut self.write).poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: S) -> Result<(), Self::Error> {
        log::info!("starting to send");
        Pin::new(&mut self.write).start_send(
            bincode::serialize(&item)
                .expect("can always serialize")
                .into(),
        )
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        log::info!("poll flush");
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        log::info!("poll close");
        Pin::new(&mut self.write).poll_close(cx)
    }
}
