//! WebSocket server that pings the browser to reload the page.
//!
//! Listens for WebSocket upgrades on a loopback port and broadcasts a "refresh"
//! string whenever the watcher publishes a new edition. Used by
//! `samizdat watch` to live-reload the browser preview.
//!
//! TODO(perf): each incoming connection is handled in its own OS thread and
//! pushed into a shared `Vec<Sender>` BEFORE the WebSocket handshake completes.
//! A local attacker holding many half-open TCP connections can exhaust threads
//! and grow the `Vec` without bound. Mitigations to consider when this matters:
//! cap concurrent connections, register the sender AFTER `accept_hdr`
//! succeeds, drop dead senders on each push, and move the accept loop onto a
//! `tokio` async runtime so threads are not allocated per connection.

use std::{
    net::SocketAddr,
    sync::{Arc, RwLock, mpsc},
    thread,
};

/// A WebSocket server that fans out a refresh signal to every connected
/// client.
pub struct RefreshSocket {
    /// Address the server is listening on.
    addr: SocketAddr,
    /// One channel per connected client. Sending `()` makes that client's
    /// connection thread push a `refresh` frame and close.
    refresh_triggers: Arc<RwLock<Vec<mpsc::Sender<()>>>>,
}

impl RefreshSocket {
    /// Binds the server on a random loopback port and spawns the accept
    /// loop. Each incoming connection runs on its own thread.
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

                    let outcome: Result<(), anyhow::Error> = (|| {
                        let stream = stream?;
                        // Reject WebSocket handshakes whose `Origin` is not a
                        // localhost page. Without this any browser tab on the
                        // user's machine (including one served by `evil.com`)
                        // can connect to the refresh port and observe rebuild
                        // pings as a side-channel.
                        // The closure's Err type is dictated by tungstenite's
                        // accept_hdr signature; boxing would require trait
                        // gymnastics for one error site.
                        #[allow(clippy::result_large_err)]
                        let mut conn = tungstenite::accept_hdr(
                            stream,
                            |req: &tungstenite::handshake::server::Request,
                             resp: tungstenite::handshake::server::Response| {
                                if !origin_is_local(req) {
                                    let msg = tungstenite::http::Response::builder()
                                        .status(403)
                                        .body(Some(
                                            "Refresh socket only accepts local origins"
                                                .to_owned(),
                                        ))
                                        .expect("can build 403");
                                    return Err(msg);
                                }
                                Ok(resp)
                            },
                        )
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                        if recv_refresh.recv().is_ok() {
                            conn.send("refresh".into())?;
                        }
                        Ok(())
                    })();

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

    /// The address the server is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Refresh every connected client and clear the client list. Failed
    /// sends are dropped; the connection thread will notice on the next
    /// poll and exit.
    pub fn trigger_refresh(&self) {
        let refresh_trigger =
            std::mem::take(&mut *self.refresh_triggers.write().expect("poisoned"));
        for send_trigger in refresh_trigger {
            send_trigger.send(()).ok();
        }
    }
}

/// True if the `Origin` header on a WebSocket upgrade looks like a
/// localhost page. A missing `Origin` is accepted (curl and native ws
/// clients omit it); only browsers that explicitly name a non-loopback
/// origin are rejected.
fn origin_is_local(req: &tungstenite::handshake::server::Request) -> bool {
    let Some(origin) = req.headers().get("Origin") else {
        return true;
    };
    let Ok(origin_str) = origin.to_str() else {
        return false;
    };
    // `Origin` is `scheme://host[:port]`. Strip the scheme and the port to
    // compare the host. Anything other than `localhost`, `127.x.x.x`, or
    // `[::1]` is rejected.
    let after_scheme = origin_str
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin_str);
    let host = after_scheme
        .rsplit_once(':')
        .map(|(host, _port)| host)
        .unwrap_or(after_scheme);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]") || host.starts_with("127.")
}
