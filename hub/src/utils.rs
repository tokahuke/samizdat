use std::net::SocketAddr;

pub fn socket_to_canonical(socket_addr: SocketAddr) -> SocketAddr {
    (socket_addr.ip().to_canonical(), socket_addr.port()).into()
}
