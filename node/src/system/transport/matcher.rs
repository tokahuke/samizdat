use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

struct MatcherInner<K: 'static + Ord + Copy + Send, T: 'static + Send> {
    expecting: BTreeMap<K, oneshot::Sender<T>>,
    arrived: BTreeMap<K, T>,
}

/// TODO: Matcher needs a limit in order to prevent flooding.
pub struct Matcher<K: 'static + Ord + Copy + Send, T: 'static + Send>(
    Arc<Mutex<MatcherInner<K, T>>>,
);

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
            let removed = inner.expecting.insert(addr, send);

            assert!(removed.is_none());

            drop(inner);

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
            let removed = inner.arrived.insert(addr, item);

            assert!(removed.is_none());

            drop(inner);

            let cloned = self.0.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(10_000)).await;
                cloned.lock().await.arrived.remove(&addr);
            });
        }
    }
}
