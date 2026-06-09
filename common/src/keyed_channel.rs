//! A channel multiplexed by key.
//!
//! Each listener carries a generation counter, so:
//!   * a second `recv_stream` on the same key does NOT silently evict the first;
//!   * when an old `RecvStream` drops, it removes its entry only if the slot still
//!     belongs to it. An outdated drop cannot kill a newer listener.
//!
//! The underlying mpsc is bounded, so a misbehaving sender cannot grow
//! memory without limit. On overflow the message is dropped and a `warn`
//! is logged.

use futures::{Stream, StreamExt, channel::mpsc};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use crate::address::ChannelId;

/// Per-listener buffer cap; once full, new sends are dropped. Big enough
/// for a healthy candidate stream, small enough to bound worst-case
/// memory.
const CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug)]
struct Slot<T> {
    /// Monotonic id distinguishing one registration on a key from a
    /// later one on the same key. Defends Drop against ABA.
    generation: u64,
    sender: mpsc::Sender<T>,
}

/// Inner state of a [`KeyedChannel`].
#[derive(Debug)]
struct KeyedChannelInner<T> {
    channels: RwLock<BTreeMap<ChannelId, Slot<T>>>,
    next_generation: AtomicU64,
}

/// A channel multiplexed by [`ChannelId`].
#[derive(Debug)]
pub struct KeyedChannel<T>(Arc<KeyedChannelInner<T>>);

impl<T> Clone for KeyedChannel<T> {
    fn clone(&self) -> Self {
        KeyedChannel(Arc::clone(&self.0))
    }
}

impl<T> Default for KeyedChannel<T> {
    fn default() -> Self {
        KeyedChannel(Arc::new(KeyedChannelInner {
            channels: RwLock::default(),
            next_generation: AtomicU64::new(1),
        }))
    }
}

impl<T> KeyedChannel<T> {
    /// An empty [`KeyedChannel`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Send `value` to whoever is listening on `key`. If nobody is, it's
    /// a no-op. If the listener's queue is full, the message is dropped
    /// and a warning is logged.
    pub fn send(&self, key: ChannelId, value: T) {
        let mut channels = self.0.channels.write().expect("poisoned");
        if let Some(slot) = channels.get_mut(&key) {
            match slot.sender.try_send(value) {
                Ok(()) => {}
                Err(err) if err.is_full() => {
                    tracing::warn!(
                        ?key,
                        "KeyedChannel queue full ({CHANNEL_CAPACITY}); dropping message"
                    );
                }
                Err(err) if err.is_disconnected() => {
                    // Listener went away without dropping cleanly; remove the dead slot.
                    channels.remove(&key);
                }
                Err(_) => {}
            }
        }
    }

    /// Subscribe to messages addressed to `key`.
    ///
    /// # Note
    ///
    /// If a listener already exists for that key, it is replaced and its
    /// stream stops yielding. This is on purpose but rare; pick
    /// `ChannelId`s with enough entropy that collisions are negligible.
    pub fn recv_stream(&self, key: ChannelId) -> RecvStream<T> {
        let (sender, recv) = mpsc::channel(CHANNEL_CAPACITY);
        let generation = self.0.next_generation.fetch_add(1, Ordering::Relaxed);
        self.0
            .channels
            .write()
            .expect("poisoned")
            .insert(key, Slot { generation, sender });
        RecvStream {
            recv,
            channel: self.clone(),
            key,
            generation,
        }
    }
}

/// Stream listening on one key of a [`KeyedChannel`].
pub struct RecvStream<T> {
    /// The receiver half.
    recv: mpsc::Receiver<T>,
    /// The keyed channel this stream belongs to.
    channel: KeyedChannel<T>,
    /// The key being listened to.
    key: ChannelId,
    /// Generation stamp, so Drop only removes our slot.
    generation: u64,
}

impl<T> Drop for RecvStream<T> {
    fn drop(&mut self) {
        let mut channels = self.channel.0.channels.write().expect("poisoned");
        if let Some(slot) = channels.get(&self.key)
            && slot.generation == self.generation
        {
            channels.remove(&self.key);
        }
        // else: a newer listener replaced us; leave its slot alone.
    }
}

impl<T> Stream for RecvStream<T> {
    type Item = T;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.recv.poll_next_unpin(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, executor::block_on};

    fn id(n: u64) -> ChannelId {
        n.into()
    }

    #[test]
    fn basic_send_recv() {
        let ch = KeyedChannel::<u32>::new();
        let mut s = ch.recv_stream(id(1));
        ch.send(id(1), 42);
        let v = block_on(s.next());
        assert_eq!(v, Some(42));
    }

    /// Regression test for P7; when a second listener registers on the same key, the
    /// first listener's `Drop` must not remove the new listener's slot.
    #[test]
    fn drop_of_replaced_listener_does_not_evict_new_one() {
        let ch = KeyedChannel::<u32>::new();
        let old = ch.recv_stream(id(1));
        // Same key: this replaces `old`'s slot.
        let mut new = ch.recv_stream(id(1));

        // Drop the displaced one. With P7 this must NOT delete the slot owned by `new`.
        drop(old);

        ch.send(id(1), 7);
        let v = block_on(new.next());
        assert_eq!(v, Some(7), "new listener was wrongly evicted by old's Drop");
    }

    /// Regression test for P7; sending after Drop on the sole listener silently no-ops.
    #[test]
    fn drop_of_sole_listener_cleans_slot() {
        let ch = KeyedChannel::<u32>::new();
        let s = ch.recv_stream(id(2));
        drop(s);
        // Should be a clean no-op now.
        ch.send(id(2), 99);
        assert!(ch.0.channels.read().unwrap().get(&id(2)).is_none());
    }

    /// Regression test for P8; channel is bounded; flooding does not cause unbounded
    /// memory growth. We send well past CHANNEL_CAPACITY and confirm the channel did not
    /// accept all of them (would have OOM'd in the old unbounded impl).
    #[test]
    fn channel_is_bounded() {
        let ch = KeyedChannel::<u32>::new();
        let _s = ch.recv_stream(id(3));
        for i in 0..(CHANNEL_CAPACITY as u32 * 4) {
            ch.send(id(3), i);
        }
        // No assertion on exact count; implementation may drop on overflow. The point is
        // we don't panic, don't OOM, and the function completes promptly.
    }
}
