use futures::future::Fuse;
use futures::prelude::*;
use quinn::{Connection, ReadToEndError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::task::JoinHandle;

/// A [`Stream`] and [`Sink`] implementation for objects that are seriaizable to be
/// passed over QUIC to (and received from) a remote peer.
pub struct BincodeOverQuic<S, R> {
    /// The QUIC connection
    connection: Connection,
    /// The current ongoing send operation.
    ongoing_send: Option<Fuse<JoinHandle<Result<(), io::Error>>>>,
    /// The current ongoing receive operation.
    ongoing_recv: Option<Fuse<JoinHandle<Result<R, io::Error>>>>,
    /// Max length that the objects can have when serialized.
    max_length: usize,
    /// A token for the data type to be sent.
    _request: PhantomData<S>,
    /// A token for the data type to be received.
    _response: PhantomData<R>,
}

impl<S, R> BincodeOverQuic<S, R>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    /// Creates a new [`BincodeOverQuic`] over an existing connection.
    pub fn new(
        connection: Connection,
        max_length: usize,
    ) -> BincodeOverQuic<S, R> {
        BincodeOverQuic {
            connection,
            ongoing_recv: None,
            ongoing_send: None,
            max_length,
            _request: PhantomData,
            _response: PhantomData,
        }
    }

    /// Restores the underlying connection.
    pub fn into_inner(self) -> Connection {
        self.connection
    }
}

/// Transforms a bincode error into an [`io::Error`].
fn bincode_error_to_io(err: Box<bincode::ErrorKind>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

/// Receives a single bincode message:
async fn recv_message<R>(connection: Connection, max_length: usize) -> Result<R, io::Error> 
where
    R: for<'a> Deserialize<'a>,
{
    let recv_stream = connection.accept_uni().await?;
    let serialized = recv_stream.read_to_end(max_length).await;

    match serialized {
        Ok(serialized) => {
            bincode::deserialize(&serialized).map_err(bincode_error_to_io)
        }
        Err(ReadToEndError::TooLong) => {
            Err(io::Error::new(io::ErrorKind::InvalidData, "too long"))
        }
        Err(ReadToEndError::Read(read)) => Err(read.into()),
    }
}

impl<S, R> Stream for BincodeOverQuic<S, R>
where
    S: 'static + Send + Serialize + Unpin,
    R: 'static + Send + Unpin + fmt::Debug + for<'a> Deserialize<'a>,
{
    type Item = Result<R, io::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        log::trace!("poll next");
        let this = self.get_mut();

        if let Some(mut ongoing_recv) = this.ongoing_recv.as_mut() {
            // Poll existing task:
            Pin::new(&mut ongoing_recv).poll(cx).map(|outcome| {
                this.ongoing_recv = None;
                Some(outcome.expect("recv task panicked"))
            })
        } else {
            // Create new task:
            let mut recv_task = tokio::spawn(recv_message(this.connection.clone(), this.max_length)).fuse();
            // Then poll:
            let polled = Pin::new(&mut recv_task).poll(cx).map(|outcome| {
                this.ongoing_recv = None;
                Some(outcome.expect("recv task panicked"))
            });

            // Set new task as existing task:
            this.ongoing_recv = Some(recv_task);

            polled
        }
    }
}

impl<S, R> Sink<S> for BincodeOverQuic<S, R>
where
    R: Unpin,
    S: 'static + Send + Unpin + fmt::Debug + Serialize,
{
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        log::trace!("poll ready");
        if let Some(ongoing_send) = self.ongoing_send.as_mut() {
            Pin::new(ongoing_send).poll(cx).map(|outcome| {
                self.ongoing_send = None;
                outcome.expect("send task panicked")
            })
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, item: S) -> Result<(), Self::Error> {
        log::trace!("starting to send");

        let this = self.get_mut();
        let connection = this.connection.clone();

        let send_task = async move {
            let mut send_stream = connection.open_uni().await?;
            let serialized = bincode::serialize(&item).expect("can serialize");
            send_stream.write_all(&serialized).await?;
            send_stream.finish().await?;

            Ok(())
        };

        if this.ongoing_send.is_some() {
            panic!("would drop ongoing send task");
        }

        this.ongoing_send = Some(tokio::spawn(send_task).fuse());

        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        log::trace!("poll flush");
        self.poll_ready(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        log::trace!("poll close");
        self.poll_ready(cx)
    }
}
