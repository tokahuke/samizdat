use futures::prelude::*;
use quinn::{Connection, IncomingUniStreams, ReadToEndError};
use serde_derive::{Deserialize, Serialize};
use std::io;

use samizdat_common::Hash;

use crate::cli;

const MAX_HEADER_LENGTH: usize = 2_048;

#[derive(Serialize, Deserialize)]
struct Header {
    hash: Hash,
    content_size: usize,
}

pub async fn recv(
    uni_streams: &mut IncomingUniStreams,
    hash: Hash,
) -> Result<Vec<u8>, crate::Error> {
    let header_stream = uni_streams
        .next()
        .await
        .ok_or_else(|| "connection dried!".to_owned())??;
    let header: Header = match header_stream.read_to_end(MAX_HEADER_LENGTH).await {
        Ok(serialized_header) => bincode::deserialize(&serialized_header)?,
        Err(ReadToEndError::TooLong) => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too long").into())
        }
        Err(ReadToEndError::Read(read)) => return Err(io::Error::from(read).into()),
    };

    if header.hash != hash {
        return Err(format!("bad hash from peer: expected {}, got {}", hash, header.hash).into());
    }

    if header.content_size > cli().max_content_size {
        return Err(format!(
            "content too big: max size is {}, advertised was {}",
            cli().max_content_size,
            header.content_size
        )
        .into());
    }

    let data_stream = uni_streams
        .next()
        .await
        .ok_or_else(|| "connection dried!".to_owned())??;
    match data_stream.read_to_end(header.content_size).await {
        Ok(data) if data.len() != header.content_size => {
            Err(format!("data length did not match content-size").into())
        }
        Ok(data) if Hash::build(&data) != hash => Err(format!(
            "bad content from peer: expected {}, got {}",
            Hash::build(&data),
            hash
        )
        .into()),
        Err(ReadToEndError::TooLong) => {
            Err(io::Error::new(io::ErrorKind::InvalidData, "too long").into())
        }
        Err(ReadToEndError::Read(read)) => Err(io::Error::from(read).into()),

        Ok(data) => Ok(data),
    }
}

pub async fn send(connection: &Connection, hash: Hash, content: &[u8]) -> Result<(), crate::Error> {
    let mut send_header = connection.open_uni().await?;
    log::info!("stream for header opened");

    let header = Header {
        hash,
        content_size: content.len(),
    };
    
    let serialized_header = bincode::serialize(&header).expect("can serialize");
    send_header
        .write_all(&serialized_header)
        .await
        .map_err(io::Error::from)?;
    log::info!("header streamed");
    send_header.finish().await.map_err(io::Error::from)?;
    log::info!("header sent");

    let mut send_data = connection.open_uni().await?;
    log::info!("stream for data opened");
    send_data
        .write_all(content)
        .await
        .map_err(io::Error::from)?;
    log::info!("data streamed");
    send_data.finish().await.map_err(io::Error::from)?;
    log::info!("data sent");

    Ok(())
}
