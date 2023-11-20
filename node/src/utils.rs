//! General utilities which don't fit anywhere else.

use std::{collections::VecDeque, net::SocketAddr};

/// An adaptor that splits the elements of another into vectors of a given size
/// (except the last). This is a "try-iterator" implementation.
pub struct Chunks<I> {
    /// The adapted iterator
    it: I,
    /// The size of the chunks.
    size: usize,
    /// Whether an error has occurred.
    is_error: bool,
    /// Whether the iterator is done iterating.
    is_done: bool,
}

impl<T, I: Iterator<Item = Result<T, crate::Error>>> Iterator for Chunks<I> {
    type Item = Result<Vec<T>, crate::Error>;
    fn next(&mut self) -> Option<Result<Vec<T>, crate::Error>> {
        if self.is_error || self.is_done {
            return None;
        }

        let mut chunk = Vec::with_capacity(self.size);
        for item in &mut self.it {
            match item {
                Ok(item) => {
                    chunk.push(item);
                    if chunk.len() == self.size {
                        return Some(Ok(chunk));
                    }
                }
                Err(error) => {
                    self.is_error = true;
                    return Some(Err(error));
                }
            }
        }

        self.is_done = true;

        Some(Ok(chunk))
    }
}

// /// An adaptor that splits the elements of another into vectors of a given size
// /// (except the last). This is a "try-iterator" implementation.
// pub fn chunks<I>(size: usize, it: I) -> Chunks<I>
// where
//     I: Iterator<Item = Result<u8, crate::Error>>,
// {
//     Chunks {
//         it,
//         size,
//         is_error: false,
//         is_done: false,
//     }
// }

/// Makes a socket address use the canonical IP form: if an IPv6 represents a tunneled
/// IPv4, then the IP will be turned into tits IPv4 address.
pub fn socket_to_canonical(socket_addr: SocketAddr) -> SocketAddr {
    (socket_addr.ip().to_canonical(), socket_addr.port()).into()
}

pub fn pop_front_chunk<T>(deque: &mut VecDeque<T>, size: usize) -> Vec<T> {
    let mut chunk = Vec::with_capacity(size);
    while let Some(item) = deque.pop_front() {
        chunk.push(item);
    }
    chunk
}

pub fn push_front_chunk<T>(deque: &mut VecDeque<T>, chunk: Vec<T>) {
    for item in chunk {
        deque.push_front(item);
    }
}
