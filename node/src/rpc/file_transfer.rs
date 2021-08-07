use futures::prelude::*;
use futures::stream;
use futures::TryStreamExt;
use quinn::{Connection, IncomingUniStreams, ReadToEndError};
use serde_derive::{Deserialize, Serialize};
use std::io;

use samizdat_common::Hash;

use crate::cache::{ObjectRef, ObjectStream};
use crate::cli;

const MAX_HEADER_LENGTH: usize = 2_048;
const MAX_STREAM_SIZE: usize = 256_000_000;

fn read_error_to_io(error: ReadToEndError) -> io::Error {
    match error {
        ReadToEndError::TooLong => io::Error::new(io::ErrorKind::InvalidData, "too long"),
        ReadToEndError::Read(read) => io::Error::from(read),
    }
}

#[derive(Serialize, Deserialize)]
struct Header {
    hash: Hash,
    content_size: usize,
    content_type: String,
}

pub async fn recv(
    uni_streams: &mut IncomingUniStreams,
    hash: Hash,
) -> Result<ObjectRef, crate::Error> {
    // Receive header from peer:
    let header_stream = uni_streams
        .next()
        .await
        .ok_or_else(|| "connection dried!".to_owned())??;
    let serialized_header = header_stream
        .read_to_end(MAX_HEADER_LENGTH)
        .await
        .map_err(read_error_to_io)?;
    let header: Header = bincode::deserialize(&serialized_header)?;

    // Check if we are getting the right hash;
    if header.hash != hash {
        return Err(format!("bad hash from peer: expected {}, got {}", hash, header.hash).into());
    }

    // Refuse if content is too big:
    if header.content_size > cli().max_content_size {
        return Err(format!(
            "content too big: max size is {}, advertised was {}",
            cli().max_content_size,
            header.content_size
        )
        .into());
    }

    // Stream the content:
    let content_stream = uni_streams
        .map_err(io::Error::from)
        .and_then(|stream| async move {
            stream
                .read_to_end(MAX_STREAM_SIZE)
                .await
                .map_err(read_error_to_io)
                .map(|buffer| {
                    stream::iter(
                        buffer
                            .into_iter()
                            .map(|byte| Ok(byte) as Result<_, io::Error>),
                    )
                })
        })
        .try_flatten()
        .map_err(crate::Error::from);

    // Build content from stream (this limits content size to the advertised amount)
    let (metadata, object) = ObjectRef::build(
        header.content_type,
        header.content_size,
        Box::pin(content_stream),
    )
    .await?;

    // Check if the peer is up to any extra sneaky tricks.
    if metadata.content_size != header.content_size {
        Err(format!(
            "actual data length did not match content-size: expected {}, got {}",
            metadata.content_size, header.content_size
        )
        .into())
    } else if object.hash != hash {
        Err(format!(
            "bad content from peer: expected {}, got {}",
            object.hash, hash
        )
        .into())
    } else {
        Ok(object)
    }
}

pub async fn send(connection: &Connection, object: ObjectRef) -> Result<(), crate::Error> {
    let mut send_header = connection.open_uni().await?;
    log::debug!("stream for header opened");

    let ObjectStream {
        iter_chunks,
        metadata,
    } = object.iter()?.expect("object exits");

    let header = Header {
        hash: object.hash,
        content_size: metadata.content_size,
        content_type: metadata.content_type,
    };

    let serialized_header = bincode::serialize(&header).expect("can serialize");
    send_header
        .write_all(&serialized_header)
        .await
        .map_err(io::Error::from)?;
    log::debug!("header streamed");
    send_header.finish().await.map_err(io::Error::from)?;
    log::debug!("header sent");

    for chunk in iter_chunks {
        let mut send_data = connection.open_uni().await?;
        log::debug!("stream for data opened");
        send_data
            .write_all(&chunk?)
            .await
            .map_err(io::Error::from)?;
        log::debug!("data streamed");
        send_data.finish().await.map_err(io::Error::from)?;
        log::debug!("data sent");
    }

    Ok(())
}
