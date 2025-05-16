//! Connection information model for tracking node connections to the hub.
//!
//! This module provides functionality for storing and managing information about
//! connected nodes, including their connection IDs and network addresses.

use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::db::Table;

use super::{Id, Indexable};

/// Information about a node's connection to the hub.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionLog {
    /// Id of this particular connection, which is also the timestamp when the
    /// connection started.
    connection_id: Id,
    /// The network address of the connected node.
    addr: SocketAddr,
}

impl Indexable for ConnectionLog {
    const TABLE: Table = Table::ConnectionLog;

    fn id(&self) -> Id {
        self.connection_id
    }
}

impl ConnectionLog {
    /// Creates a new connection info entry with the given socket address.
    ///
    /// # Arguments
    /// * `addr` - The network address of the connected node
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            connection_id: Id::generate(),
            addr,
        }
    }
}
