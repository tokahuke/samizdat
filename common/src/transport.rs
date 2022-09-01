use futures::future::Fuse;
use futures::prelude::*;
use quinn::{Connection, ConnectionError, IncomingUniStreams, ReadToEndError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::task::JoinHandle;

pub struct BincodeOverQuic<S, R> {
    connection: Connection,
    incoming: IncomingUniStreams,
    ongoing_send: Option<Fuse<JoinHandle<Result<(), io::Error>>>>,
    ongoing_recv: Option<Fuse<JoinHandle<Result<R, io::Error>>>>,
    max_length: usize,
    _request: PhantomData<S>,
    _response: PhantomData<R>,
}

impl<S, R> BincodeOverQuic<S, R>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    pub fn new(
        connection: Connection,
        incoming: IncomingUniStreams,
        max_length: usize,
    ) -> BincodeOverQuic<S, R> {
        BincodeOverQuic {
            connection,
            incoming,
            ongoing_recv: None,
            ongoing_send: None,
            max_length,
            _request: PhantomData,
            _response: PhantomData,
        }
    }

    pub fn into_inner(self) -> (Connection, IncomingUniStreams) {
        (self.connection, self.incoming)
    }
}

fn bincode_error_to_io(err: Box<bincode::ErrorKind>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
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
            Pin::new(&mut ongoing_recv).poll(cx).map(|outcome| {
                this.ongoing_recv = None;
                Some(outcome.expect("recv task panicked"))
            })
        } else {
            match Pin::new(&mut this.incoming).poll_next(cx) {
                Poll::Ready(Some(Ok(recv_stream))) => {
                    let max_length = this.max_length;
                    let recv_task = tokio::spawn(async move {
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
                    })
                    .fuse();

                    this.ongoing_recv = Some(recv_task);

                    Pin::new(this).poll_next(cx)
                }
                Poll::Ready(Some(Err(ConnectionError::ApplicationClosed(close))))
                    if close.error_code.into_inner() == 0 =>
                {
                    Poll::Ready(None)
                }
                Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err.into()))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
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
        let open_uni = this.connection.open_uni();

        let send_task = async move {
            let mut send_stream = open_uni.await?;
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
