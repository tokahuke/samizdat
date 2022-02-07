//! RPC implementation for the Node. This RPC is called by the hubs to trigger object resoution.

use futures::prelude::*;
use std::sync::Arc;
use tarpc::context;

use samizdat_common::cipher::TransferCipher;
use samizdat_common::rpc::*;
use samizdat_common::ChannelAddr;
use samizdat_common::Hash;

use crate::models::{CollectionItem, Edition, ObjectRef, SeriesRef, SubscriptionRef};

use super::file_transfer;
use super::transport::ChannelManager;

#[derive(Clone)]
pub struct NodeServer {
    pub channel_manager: Arc<ChannelManager>,
}

impl NodeServer {
    async fn resolve_object(self, resolution: Arc<Resolution>) -> ResolutionResponse {
        log::info!("got object {:?}", resolution);

        let object = match ObjectRef::find(&resolution.content_riddle) {
            Some(object) if !object.is_draft().unwrap_or(true) => object,
            Some(_) => {
                log::info!("Hash found but object is draft");
                return ResolutionResponse::NOT_FOUND;
            }
            None => {
                log::info!("Hash not found for resolution");
                return ResolutionResponse::NOT_FOUND;
            }
        };

        let hash = *object.hash();

        log::info!("Found hash {}", hash);
        let peer_addr = match resolution.message_riddle.resolve::<ChannelAddr>(&hash) {
            Some(socket_addr) => socket_addr,
            None => {
                log::warn!("Failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::FOUND;
            }
        };

        log::info!("Found peer at {peer_addr}");

        tokio::spawn(
            async move {
                log::info!("Starting task to transfer object {} to {}", hash, peer_addr);
                let (sender, _receiver) = self.channel_manager.initiate(peer_addr).await?;
                file_transfer::send_object(&sender, &object).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("Failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        ResolutionResponse::FOUND
    }

    async fn resolve_item(self, resolution: Arc<Resolution>) -> ResolutionResponse {
        log::info!("got item {:?}", resolution);

        let item = match CollectionItem::find(&resolution.content_riddle) {
            Ok(Some(item)) if !item.is_draft => item,
            Ok(Some(_)) => {
                log::info!("hash found, but item is draft");
                return ResolutionResponse::NOT_FOUND;
            }
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
        let peer_addr = match resolution.message_riddle.resolve::<ChannelAddr>(&hash) {
            Some(socket_addr) => socket_addr,
            None => {
                log::warn!("failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::FOUND;
            }
        };

        log::info!("found peer at {}", peer_addr);

        tokio::spawn(
            async move {
                let (sender, _receiver) = self.channel_manager.initiate(peer_addr).await?;
                file_transfer::send_item(&sender, item).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        ResolutionResponse::FOUND
    }
}

#[tarpc::server]
impl Node for NodeServer {
    async fn resolve(self, _: context::Context, resolution: Arc<Resolution>) -> ResolutionResponse {
        match resolution.kind {
            QueryKind::Object => self.resolve_object(resolution).await,
            QueryKind::Item => self.resolve_item(resolution).await,
        }
    }

    async fn resolve_latest(
        self,
        _: context::Context,
        latest: Arc<LatestRequest>,
    ) -> Option<LatestResponse> {
        if let Some(series) = SeriesRef::find(&latest.key_riddle) {
            let editions = series.get_editions();
            match editions.as_ref().map(|editions| editions.first()) {
                Ok(None) => None,
                Ok(Some(latest)) if latest.is_draft() => None,
                Ok(Some(latest)) => {
                    let cipher_key = latest.public_key().hash();
                    let rand = Hash::rand();
                    let cipher = TransferCipher::new(&cipher_key, &rand);

                    Some(LatestResponse {
                        rand,
                        series: cipher.encrypt_opaque(latest),
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

    async fn announce_edition(self, _: context::Context, announcement: Arc<EditionAnnouncement>) {
        log::info!("Sending announcement to hub");
        if let Some(subscription) = SubscriptionRef::find(&announcement.key_riddle) {
            let cipher = TransferCipher::new(&subscription.public_key.hash(), &announcement.rand);

            let try_refresh = async move {
                let edition: Edition = announcement.edition.clone().decrypt_with(&cipher)?;

                if !edition.is_valid() {
                    log::warn!("an invalid edition was announced: {:?}", edition);
                    return Ok(());
                }

                if subscription.must_refresh()? {
                    subscription.refresh(edition).await
                } else {
                    Ok(())
                }
            };

            tokio::spawn(async move {
                // Sleep a random amount so as not for eeeeverybody to ask for the same items at
                // the same time.
                tokio::time::sleep(std::time::Duration::from_secs_f32(rand::random())).await;
                if let Err(err) = try_refresh.await {
                    log::warn!("{}", err);
                }
            });
        }
    }
}
