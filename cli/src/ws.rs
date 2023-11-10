//! Serves a very simple WebSocket server for triggering page refresh.

use std::{
    net::SocketAddr,
    sync::{mpsc, Arc, RwLock},
    thread,
};

pub struct RefreshSocket {
    addr: SocketAddr,
    refresh_triggers: Arc<RwLock<Vec<mpsc::Sender<()>>>>,
}

impl RefreshSocket {
    pub fn init() -> Result<RefreshSocket, anyhow::Error> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let refresh_triggers: Arc<RwLock<Vec<_>>> = Arc::default();
        let addr = listener.local_addr()?;

        let thread_refresh_triggers = refresh_triggers.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let thread_refresh_triggers = thread_refresh_triggers.clone();
                thread::spawn(move || {
                    let (send_refresh, recv_refresh) = mpsc::channel();
                    thread_refresh_triggers
                        .write()
                        .expect("poisoned")
                        .push(send_refresh);

                    let outcome: Result<(), anyhow::Error> = try {
                        let mut conn = tungstenite::accept(stream?)?;
                        if recv_refresh.recv().is_ok() {
                            conn.send("refresh".into())?;
                        } else {
                            println!("Gooba gooba");
                        }
                    };

                    if let Err(err) = outcome {
                        println!("ERROR: running websocket: {err}");
                    }
                });
            }
        });

        Ok(RefreshSocket {
            addr,
            refresh_triggers,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn trigger_refresh(&self) {
        let refresh_trigger =
            std::mem::take(&mut *self.refresh_triggers.write().expect("poisoned"));
        for send_trigger in refresh_trigger {
            send_trigger.send(()).ok();
        }
    }
}
