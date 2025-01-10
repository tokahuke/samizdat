//! General utilities which don't fit anywhere else.
//!
//! This module provides utility functions and types for common operations across this crate,
//! including chunk-based iteration over collections and socket address manipulation.

use std::{collections::VecDeque, net::SocketAddr};

/// An adaptor that splits the elements of another iterator into vectors of a given size.
///
/// This is a "try-iterator" implementation that handles Result types, collecting elements
/// into chunks until either the chunk size is reached or an error occurs.
pub struct Chunks<I> {
    /// The adapted iterator
    it: I,
    /// The size of the chunks to be produced
    size: usize,
    /// Whether an error has occurred during iteration
    is_error: bool,
    /// Whether the iterator has completed
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
