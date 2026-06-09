//! Utilities to maintain connectivity in an uncertain world.

use num_derive::FromPrimitive;
use num_traits::FromPrimitive as _;
use serde_derive::Serialize;
use std::{
    fmt::Display,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use tokio::{
    sync::{RwLock, RwLockReadGuard},
    task::JoinHandle,
    time::{Duration, sleep},
};

/// Lifecycle state of a wrapped reconnecting connection.
#[derive(Debug, FromPrimitive, Serialize)]
pub enum ConnectionStatus {
    /// First connect attempt is in flight.
    Connecting,
    /// Connection is live.
    Connected,
    /// Live connection just dropped; supervisor is about to retry.
    Failing,
    /// Retry attempt is in flight.
    Reconnecting,
}

/// Exponential backoff. Just that.
pub fn exponential_backoff(start: Duration, max: Duration) -> impl FnMut() -> Duration {
    let mut delay = start;
    move || {
        let this_delay = delay;
        delay = if 2 * delay > max { max } else { 2 * delay };
        this_delay
    }
}

/// A structure that tries to re-establish a connection, come hell or high water.
pub struct Reconnect<T> {
    /// The current active connection.
    current: Arc<RwLock<Option<T>>>,
    status: Arc<AtomicU8>,
    /// The task that monitors the connections and reconnects if necessary.
    _reconnect: JoinHandle<()>,
}

impl<T: 'static + Send + Sync> Reconnect<T> {
    /// Inits the reconnector.
    pub async fn init<C, CFut, R, Bf, B, E>(
        mut connect: C,
        mut backoff_factory: Bf,
    ) -> Result<Reconnect<T>, E>
    where
        C: 'static + Send + FnMut() -> CFut,
        CFut: Send + Future<Output = Result<(T, R), E>>,
        E: Display + Send,
        R: 'static + Send + Future<Output = ()>,
        Bf: 'static + Send + FnMut() -> B,
        B: Send + FnMut() -> Duration,
    {
        let current = Arc::new(RwLock::new(None));
        let status = Arc::new(AtomicU8::new(ConnectionStatus::Connecting as u8));

        let task_current = current.clone();
        let task_status = status.clone();
        let reconnect = tokio::spawn(async move {
            loop {
                tracing::info!("connection reset triggered");

                // Publish "disconnected" so concurrent readers get
                // `None` (and respond "Not yet connected" promptly)
                // instead of parking on the writer while we sleep
                // through backoff. The write is brief; subsequent
                // attempts run without any lock held.
                *task_current.write().await = None;

                let mut backoff = backoff_factory();
                let (connection, reset) = 'inner: loop {
                    match connect().await {
                        Ok(success) => {
                            tracing::info!("connect attempt succeeded.");
                            task_status.store(ConnectionStatus::Connected as u8, Ordering::Relaxed);
                            break 'inner success;
                        }
                        Err(err) => {
                            tracing::warn!("connect attempt failed: {}", err);
                            task_status.store(ConnectionStatus::Failing as u8, Ordering::Relaxed);
                            sleep(backoff()).await;
                        }
                    }
                };

                // Briefly take the writer to publish the new
                // connection, then drop it before awaiting reset so
                // queries against the now-live connection get the
                // read lock immediately.
                *task_current.write().await = Some(connection);
                reset.await;
                task_status.store(ConnectionStatus::Reconnecting as u8, Ordering::Relaxed);
            }
        });

        Ok(Reconnect {
            current,
            status,
            _reconnect: reconnect,
        })
    }

    pub fn status(&self) -> ConnectionStatus {
        ConnectionStatus::from_u8(self.status.load(Ordering::Relaxed))
            .expect("Is a valid representation")
    }

    /// Gets the current active connection.
    pub async fn get(&'_ self) -> RwLockReadGuard<'_, Option<T>> {
        self.current.read().await
    }
}
