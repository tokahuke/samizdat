use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

struct MatcherInner<K: 'static + Ord + Copy + Send + Display, T: 'static + Send> {
    expecting: BTreeMap<K, oneshot::Sender<T>>,
    arrived: BTreeMap<K, T>,
}

/// TODO: Matcher needs a limit in order to prevent flooding.
///
/// Every call to `expect`/`arrive` `tokio::spawn`s a 10-second sleeping cleanup task.
/// A noisy peer can drive these via streamed channel ids over a single QUIC connection;
/// each spawn is cheap individually but unbounded under sustained flooding. If this ever
/// shows up under load, switch to a single `tokio::time::DelayQueue` shared across the
/// matcher and bound the map size with backpressure on insert.
pub struct Matcher<K: 'static + Ord + Copy + Send + Display, T: 'static + Send>(
    Arc<Mutex<MatcherInner<K, T>>>,
);

impl<K: 'static + Ord + Copy + Send + Display, T: 'static + Send> Default for Matcher<K, T> {
    fn default() -> Matcher<K, T> {
        Matcher(Arc::new(Mutex::new(MatcherInner {
            expecting: BTreeMap::new(),
            arrived: BTreeMap::new(),
        })))
    }
}

impl<K: 'static + Ord + Copy + Send + Display, T: 'static + Send> Clone for Matcher<K, T> {
    fn clone(&self) -> Matcher<K, T> {
        Matcher(self.0.clone())
    }
}

impl<K: 'static + Ord + Copy + Send + Display, T: 'static + Send> Matcher<K, T> {
    pub async fn expect(&self, addr: K) -> Option<T> {
        let mut inner = self.0.lock().await;
        if let Some(item) = inner.arrived.remove(&addr) {
            Some(item)
        } else {
            let (send, recv) = oneshot::channel();
            if let Some(displaced) = inner.expecting.insert(addr, send) {
                // Two waiters on the same key collided. Don't panic; it's reachable
                // via random ChannelId allocation (or a malicious peer replaying ids on
                // the network path). Drop the older waiter; it gets `None` from `.recv`.
                if !displaced.is_closed() {
                    tracing::warn!(
                        "Matcher::expect: displaced an existing waiter for key {addr}; \
                         older caller will receive None"
                    );
                }
            }

            drop(inner);

            let cloned = self.0.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(10_000)).await;
                if cloned.lock().await.expecting.remove(&addr).is_some() {
                    tracing::warn!("Key {addr}, which was expected, never arrived");
                }
            });

            recv.await.ok()
        }
    }

    pub async fn arrive(&self, addr: K, item: T) {
        let mut inner = self.0.lock().await;
        if let Some(send) = inner.expecting.remove(&addr) {
            send.send(item).ok();
        } else {
            if inner.arrived.insert(addr, item).is_some() {
                // A second item arrived for an already-pending key. Drop the prior one
                // (now overwritten) rather than panic; this path is reachable from
                // peer-supplied channel ids.
                tracing::warn!(
                    "Matcher::arrive: overwrote an unclaimed prior arrival for key {addr}"
                );
            }

            drop(inner);

            let cloned = self.0.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(10_000)).await;
                if cloned.lock().await.arrived.remove(&addr).is_some() {
                    tracing::warn!("Key {addr}, which arrived, was never expected");
                }
            });
        }
    }
}
