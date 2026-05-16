//! General utilities which don't fit anywhere else.
//!
//! Provides socket-address canonicalisation helpers and small VecDeque adaptors used
//! by the chunk-based file transfer pipeline.

use std::{collections::VecDeque, net::SocketAddr};

/// Makes a socket address use the canonical IP form.
///
/// If an IPv6 address represents a tunneled IPv4 address, it will be converted to its IPv4
/// form while preserving the port number.
pub fn socket_to_canonical(socket_addr: SocketAddr) -> SocketAddr {
    (socket_addr.ip().to_canonical(), socket_addr.port()).into()
}

/// Removes and returns up to `size` elements from the front of a VecDeque.
pub fn pop_front_chunk<T>(deque: &mut VecDeque<T>, size: usize) -> Vec<T> {
    let mut chunk = Vec::with_capacity(size);
    while let Some(item) = deque.pop_front() {
        chunk.push(item);
    }
    chunk
}

/// Adds multiple elements to the front of a VecDeque.
pub fn push_front_chunk<T>(deque: &mut VecDeque<T>, chunk: Vec<T>) {
    for item in chunk {
        deque.push_front(item);
    }
}
