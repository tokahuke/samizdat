//! WebSocket server for page refresh functionality.
//!
//! This module implements a simple WebSocket server that listens for connections and sends
//! refresh signals to connected clients. It's used to automatically refresh web pages when
//! a new edition is created by the subcommand `samizdat watch`.

use std::{
    net::SocketAddr,
    sync::{mpsc, Arc, RwLock},
    thread,
};

/// A WebSocket server that handles page refresh signals.
///
/// Maintains a list of connected clients and provides functionality to trigger page
/// refreshes across all active connections.
pub struct RefreshSocket {
    /// The socket address where the server is listening
    addr: SocketAddr,
    /// List of channels to send refresh signals to connected clients
    refresh_triggers: Arc<RwLock<Vec<mpsc::Sender<()>>>>,
}

impl RefreshSocket {
    /// Initializes a new WebSocket server on a random port.
    ///
    /// Creates a TCP listener and spawns a background thread to handle incoming WebSocket
    /// connections. Each connection is handled in its own thread.
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

    /// Returns the socket address where the server is listening.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Triggers a page refresh for all connected clients.
    ///
    /// Sends a refresh signal to each connected client and removes disconnected clients
    /// from the list of active connections.
    pub fn trigger_refresh(&self) {
        let refresh_trigger =
            std::mem::take(&mut *self.refresh_triggers.write().expect("poisoned"));
        for send_trigger in refresh_trigger {
            send_trigger.send(()).ok();
        }
    }
}
