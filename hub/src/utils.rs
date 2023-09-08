//! General utilities which don’t fit anywhere else.

use std::net::SocketAddr;

/// Makes a socket address use the canonical IP form: if an IPv6 represents a tunneled
/// IPv4, then the IP will be turned into tits IPv4 address.
pub fn socket_to_canonical(socket_addr: SocketAddr) -> SocketAddr {
    (socket_addr.ip().to_canonical(), socket_addr.port()).into()
}
