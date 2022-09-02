//! A channel that can be multiplexed with a key. 

use futures::{channel::mpsc, Stream, StreamExt};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{Arc, RwLock},
    task::{Context, Poll},
};

use crate::rpc::CandidateChannelId;

/// A channel that can be multiplexed with a key.
#[derive(Debug)]
struct KeyedChannelInner<T> {
    channels: RwLock<BTreeMap<CandidateChannelId, mpsc::UnboundedSender<T>>>,
}

/// A channel that can be multiplexed with a key.
#[derive(Debug, Clone)]
pub struct KeyedChannel<T>(Arc<KeyedChannelInner<T>>);

// TODO: this Clone bound is a dumb hack to make compiler happy.
impl<T: Clone> KeyedChannel<T> {
    /// Creates a new [`KeyedChannel`].
    pub fn new() -> Self {
        KeyedChannel(Arc::new(KeyedChannelInner {
            channels: RwLock::default(),
        }))
    }

    /// Sends an item to a specified address. If nobody is listening on the specified key,
    /// nothing happens.
    pub fn send(&self, key: CandidateChannelId, value: T) {
        self.0
            .channels
            .read()
            .expect("poisoned")
            .get(&key)
            .map(|sender| sender.unbounded_send(value));
    }

    /// Listens to a given key. 
    /// 
    /// # Note
    /// 
    /// If there already exists a listener on that key, the existing listener will be
    /// dropped.
    pub fn recv_stream(&self, key: CandidateChannelId) -> RecvStream<T> {
        let (sender, recv) = mpsc::unbounded();
        self.0
            .channels
            .write()
            .expect("poisoned")
            .insert(key, sender);
        RecvStream {
            recv,
            channel: Self::clone(&self),
            key,
        }
    }
}

/// A stream listening to a specific key on a [`KeyedChannel`].
pub struct RecvStream<T> {
    /// The receiver stream.
    recv: mpsc::UnboundedReceiver<T>,
    /// The keyed channel this stream belongs to.
    channel: KeyedChannel<T>,
    /// The key being listened to.
    key: CandidateChannelId,
}

impl<T> Drop for RecvStream<T> {
    fn drop(&mut self) {
        self.channel
            .0
            .channels
            .write()
            .expect("poisoned")
            .remove(&self.key);
    }
}

impl<T> Stream for RecvStream<T> {
    type Item = T;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.recv.poll_next_unpin(cx)
    }
}
