use std::fmt::Display;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

// pub enum ConnectionStatus {
//     Connected,
//     Reconnecting,
// }

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
    current: Arc<RwLock<T>>,
    _reconnect: JoinHandle<()>,
}

impl<T: 'static + Send + Sync> Reconnect<T> {
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
        let (connection, reset) = connect().await?;
        let current = Arc::new(RwLock::new(connection));

        let current_task = current.clone();
        let reconnect = tokio::spawn(async move {
            reset.await;

            loop {
                log::info!("connection reset triggered");

                let mut backoff = backoff_factory();
                let mut lock = current_task.write().await;
                let (connection, reset) = loop {
                    match connect().await {
                        Ok(success) => {
                            log::info!("connect attempt succeeded.");
                            break success;
                        }
                        Err(err) => {
                            log::warn!("connect attempt failed: {}", err);
                            sleep(backoff()).await;
                        }
                    }
                };

                *lock = connection;
                drop(lock);
                reset.await;
            }
        });

        Ok(Reconnect {
            current,
            _reconnect: reconnect,
        })
    }

    // pub fn status(&self) -> ConnectionStatus {
    //     if self.current.try_read().is_ok() {
    //         ConnectionStatus::Connected
    //     } else {
    //         ConnectionStatus::Reconnecting
    //     }
    // }

    pub async fn get<'a>(&'a self) -> RwLockReadGuard<'a, T> {
        self.current.read().await
    }
}
