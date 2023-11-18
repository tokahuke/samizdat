#![allow(unused)]

// //! Protocol for information transfer between peers.

// mod messages;
// pub mod v2;

use std::time::Duration;

use samizdat_common::Hash;

use crate::models::ContentStream;
use crate::models::{CollectionItem, ObjectMetadata, ObjectRef};

use super::{ChannelReceiver, ChannelSender};

use super::messages::{ItemMessage, Message, NonceMessage, ObjectMessage, ProceedMessage};

/// Represents an object in the process of being received. The object to which this
/// struct corresponds most likely does not exist in the database.
pub struct ReceivedObject {
    /// The received, but not verified metadata.
    metadata: ObjectMetadata,
    /// A stream for the incoming content. The content is streamed from the local
    /// database. Therefore it is "proper for consumption".
    content_stream: ContentStream,
    /// The object handle for the soon-to-be object.
    object_ref: ObjectRef,
    /// The time that took for the object to be resolved.
    query_duration: Duration,
}

impl ReceivedObject {
    /// A stream for the incoming content. The content is streamed from the local
    /// database. Therefore it is "proper for consumption".
    pub fn into_content_stream(self) -> ContentStream {
        self.content_stream
    }

    /// The object handle for the soon-to-be object.
    pub fn object_ref(&self) -> ObjectRef {
        self.object_ref.clone()
    }

    /// The received, but not verified metadata.
    pub fn metadata(&self) -> &ObjectMetadata {
        &self.metadata
    }

    /// The time that took for the object to be resolved.
    pub fn query_duration(&self) -> Duration {
        self.query_duration
    }
}

/// Receives the object from a channel.
pub async fn recv_object(
    _sender: ChannelSender,
    mut receiver: ChannelReceiver,
    hash: Hash,
    query_duration: Duration,
) -> Result<ReceivedObject, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, hash).await?;
    log::info!("receiving object header");
    let header = ObjectMessage::recv(&mut receiver, &transfer_cipher).await?;
    let metadata = header.metadata.clone();
    log::info!("receiving data");
    let content_stream = header.recv_data(receiver, hash, query_duration)?;

    log::info!("done receiving object");

    Ok(ReceivedObject {
        metadata,
        content_stream,
        object_ref: ObjectRef::new(hash),
        query_duration,
    })
}

/// Sends an object to a channel.
pub async fn send_object(
    sender: ChannelSender,
    _receiver: ChannelReceiver,
    object: &ObjectRef,
) -> Result<(), crate::Error> {
    let header = ObjectMessage::for_object(object)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(&sender, *object.hash()).await?;
    log::info!("sending object header");
    header.send(&sender, &transfer_cipher).await?;
    log::info!("sending data");
    header.send_data(&sender, object).await?;

    log::info!("done sending object");

    Ok(())
}

/// Represents an item in the process of being received. It can correspond either to a
/// fresh new object or to an existing object in the database.
pub enum ReceivedItem {
    /// The item resolved to a fresh new object not present in th database.
    NewObject(ReceivedObject),
    /// The item resolved to an existing object.
    ExistingObject(ObjectRef),
}

impl ReceivedItem {
    pub fn object_ref(&self) -> ObjectRef {
        match self {
            Self::NewObject(n) => n.object_ref(),
            Self::ExistingObject(e) => e.clone(),
        }
    }
}

/// Receive a collection item from a channel. Returns `Ok(None)` if the item object is
/// perceived to already exist in the database.
pub async fn recv_item(
    sender: ChannelSender,
    mut receiver: ChannelReceiver,
    locator_hash: Hash,
    query_duration: Duration,
) -> Result<ReceivedItem, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, locator_hash).await?;
    log::info!("receiving item header");
    let header = ItemMessage::recv(&mut receiver, &transfer_cipher).await?;

    // No tricks!
    let locator_hash_from_peer = header.item.locator().hash();
    if locator_hash_from_peer != locator_hash {
        return Err(crate::Error::Message(format!(
            "bad item from peer: expected {}, got {}",
            locator_hash, locator_hash_from_peer,
        )));
    }

    // This checks proof validity:
    let object_ref = header.item.object()?;

    // Go away if you already have what you wanted:
    if object_ref.exists()? {
        // Do not attempt to create a `ReceivedObject, because it will attempt to reinsert
        // the object in the database.
        log::info!("Object {} exists. Ending transmission", object_ref.hash());
        // Need to reaffirm the collection-object connection (e.g. the object is there,
        // but is part of another collection and the link is not yet established):
        header.item.insert()?;
        ProceedMessage::Cancel
            .send(&sender, &transfer_cipher)
            .await?;
        return Ok(ReceivedItem::ExistingObject(object_ref));
    } else {
        ProceedMessage::Proceed
            .send(&sender, &transfer_cipher)
            .await?;
    }

    header.item.insert()?;

    log::info!("receiving data");
    let metadata = header.object_header.metadata.clone();
    let content_stream =
        header
            .object_header
            .recv_data(receiver, *object_ref.hash(), query_duration)?;

    log::info!("done receiving item");

    Ok(ReceivedItem::NewObject(ReceivedObject {
        metadata,
        content_stream,
        object_ref,
        query_duration,
    }))
}

/// Sends a collection item to a channel.
pub async fn send_item(
    sender: ChannelSender,
    mut receiver: ChannelReceiver,
    item: CollectionItem,
) -> Result<(), crate::Error> {
    let object = item.object()?;
    let hash = item.locator().hash();
    let header = ItemMessage::for_item(item)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(&sender, hash).await?;
    log::info!("sending item header");
    header.send(&sender, &transfer_cipher).await?;

    log::info!("Receiving proceed message");
    let proceed = ProceedMessage::recv(&mut receiver, &transfer_cipher).await?;

    match proceed {
        ProceedMessage::Proceed => {
            log::info!("sending data");
            header.object_header.send_data(&sender, &object).await?;
        }
        ProceedMessage::Cancel => {
            log::info!("no need to send data");
        }
    }

    log::info!("done sending object");

    Ok(())
}
