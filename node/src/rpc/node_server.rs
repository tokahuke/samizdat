use futures::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;

use samizdat_common::cipher::TransferCipher;
use samizdat_common::rpc::*;
use samizdat_common::Hash;

use crate::models::{CollectionItem, ObjectRef, SeriesRef};

use super::connection_manager::{ConnectionManager, DropMode};
use super::file_transfer;

#[derive(Clone)]
pub struct NodeServer {
    pub connection_manager: Arc<ConnectionManager>,
}

#[tarpc::server]
impl Node for NodeServer {
    async fn resolve_object(
        self,
        _: context::Context,
        resolution: Arc<Resolution>,
    ) -> ResolutionResponse {
        log::info!("got object {:?}", resolution);

        let object = match ObjectRef::find(&resolution.content_riddle) {
            Some(object) => object,
            None => {
                log::info!("hash not found for resolution");
                return ResolutionResponse::NOT_FOUND;
            }
        };

        // Code smell?
        let hash = object.hash;

        log::info!("found hash {}", object.hash);
        let peer_addr = match resolution.message_riddle.resolve::<SocketAddr>(&hash) {
            Some(socket_addr) => socket_addr,
            None => {
                log::warn!("failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::FOUND;
            }
        };

        log::info!("found peer at {}", peer_addr);

        tokio::spawn(
            async move {
                let new_connection = self
                    .connection_manager
                    .punch_hole_to(peer_addr, DropMode::DropIncoming)
                    .await?;
                file_transfer::send_object(&new_connection.connection, &object).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        return ResolutionResponse::FOUND;
    }

    async fn resolve_item(
        self,
        _: context::Context,
        resolution: Arc<Resolution>,
    ) -> ResolutionResponse {
        log::info!("got item {:?}", resolution);

        let item = match CollectionItem::find(&resolution.content_riddle) {
            Ok(Some(item)) => item,
            Ok(None) => {
                log::info!("hash not found for resolution");
                return ResolutionResponse::NOT_FOUND;
            }
            Err(e) => {
                log::error!("error looking for hash: {}", e);
                return ResolutionResponse::NOT_FOUND;
            }
        };

        // Code smell?
        let hash = item.locator().hash();

        log::info!("found hash {}", hash);
        let peer_addr = match resolution.message_riddle.resolve::<SocketAddr>(&hash) {
            Some(socket_addr) => socket_addr,
            None => {
                log::warn!("failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::FOUND;
            }
        };

        log::info!("found peer at {}", peer_addr);

        tokio::spawn(
            async move {
                let new_connection = self
                    .connection_manager
                    .punch_hole_to(peer_addr, DropMode::DropIncoming)
                    .await?;
                file_transfer::send_item(&new_connection.connection, item).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        return ResolutionResponse::FOUND;
    }

    async fn resolve_latest(
        self,
        _: context::Context,
        latest: Arc<LatestRequest>,
    ) -> Option<LatestResponse> {
        if let Some(series) = SeriesRef::find(&latest.key_riddle) {
            match series.get_latest_fresh() {
                Ok(None) => None,
                Ok(Some(mut latest)) => {
                    let cipher_key = latest.public_key().hash();
                    let rand = Hash::rand();
                    let cipher = TransferCipher::new(&cipher_key, &rand);
                    latest.erase_freshness();

                    Some(LatestResponse {
                        rand,
                        series: cipher.encrypt_opaque(&latest),
                    })
                }
                Err(err) => {
                    log::warn!("{}", err);
                    None
                }
            }
        } else {
            None
        }
    }
}
