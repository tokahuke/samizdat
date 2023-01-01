//! RPC implementation for the Node. This RPC is called by the hubs to trigger object resolution.

use futures::prelude::*;
use std::sync::Arc;
use tarpc::context;

use samizdat_common::address::{ChannelAddr, ChannelId};
use samizdat_common::cipher::TransferCipher;
use samizdat_common::keyed_channel::KeyedChannel;
use samizdat_common::rpc::*;
use samizdat_common::{Hash, Riddle};

use crate::models::{CollectionItem, Edition, Identity, ObjectRef, SeriesRef, SubscriptionRef};

use super::file_transfer;
use super::transport::ChannelManager;

#[derive(Clone)]
pub struct NodeServer {
    pub channel_manager: Arc<ChannelManager>,
    pub candidate_channels: KeyedChannel<Candidate>,
}

impl NodeServer {
    async fn resolve_object(self, resolution: Arc<Resolution>) -> ResolutionResponse {
        log::info!("got object {resolution:?}");

        let content_riddle = if let Some(content_riddle) = resolution.content_riddles.first() {
            content_riddle
        } else {
            return ResolutionResponse::EmptyResolution;
        };
        let object = match ObjectRef::find(content_riddle) {
            Ok(Some(object)) if !object.is_draft().unwrap_or(true) => object,
            Ok(Some(_)) => {
                log::info!("Hash found but object is draft");
                return ResolutionResponse::NotFound;
            }
            Ok(None) => {
                log::info!("Hash not found for resolution");
                return ResolutionResponse::NotFound;
            }
            Err(err) => {
                log::error!("Error while looking for object {resolution:?}: {err}");
                return ResolutionResponse::NotFound;
            }
        };

        let hash = *object.hash();

        log::info!("Found hash {}", hash);
        let peer_addr = match resolution
            .location_message_riddle
            .resolve::<ChannelAddr>(&hash)
        {
            Some(socket_addr) => socket_addr,
            None => {
                log::warn!("Failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::NotFound;
            }
        };

        log::info!("Found peer at {peer_addr}");

        tokio::spawn(
            async move {
                log::info!("Starting task to transfer object {} to {}", hash, peer_addr);
                let (sender, receiver) = self.channel_manager.initiate(peer_addr).await?;
                file_transfer::send_object(sender, receiver, &object).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("Failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        ResolutionResponse::Found(
            resolution
                .validation_nonces
                .iter()
                .map(|&nonce| Riddle::new_with_nonce(&hash, nonce))
                .collect(),
        )
    }

    async fn resolve_item(self, resolution: Arc<Resolution>) -> ResolutionResponse {
        log::info!("got item {:?}", resolution);

        let content_riddle = if let Some(content_riddle) = resolution.content_riddles.first() {
            content_riddle
        } else {
            return ResolutionResponse::EmptyResolution;
        };
        let item = match CollectionItem::find(&content_riddle) {
            Ok(Some(item)) if !item.is_draft => item,
            Ok(Some(_)) => {
                log::info!("hash found, but item is draft");
                return ResolutionResponse::NotFound;
            }
            Ok(None) => {
                log::info!("hash not found for resolution");
                return ResolutionResponse::NotFound;
            }
            Err(e) => {
                log::error!("error looking for hash: {}", e);
                return ResolutionResponse::NotFound;
            }
        };

        // Code smell?
        let hash = item.locator().hash();

        log::info!("found hash {}", hash);
        let peer_addr = match resolution
            .location_message_riddle
            .resolve::<ChannelAddr>(&hash)
        {
            Some(socket_addr) => socket_addr,
            None => {
                log::warn!("failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::NotFound;
            }
        };

        log::info!("found peer at {}", peer_addr);

        tokio::spawn(
            async move {
                let (sender, receiver) = self.channel_manager.initiate(peer_addr).await?;
                file_transfer::send_item(sender, receiver, item).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        ResolutionResponse::Found(
            resolution
                .validation_nonces
                .iter()
                .map(|&nonce| Riddle::new_with_nonce(&hash, nonce))
                .collect(),
        )
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

    async fn recv_candidate(
        self,
        _: context::Context,
        candidate_channel: ChannelId,
        candidate: Candidate,
    ) {
        self.candidate_channels.send(candidate_channel, candidate);
    }

    async fn get_edition(
        self,
        _: context::Context,
        latest: Arc<EditionRequest>,
    ) -> Vec<EditionResponse> {
        log::info!("got {latest:?}");

        let maybe_response = if let Some(series) = SeriesRef::find(&latest.key_riddle).transpose() {
            match series.and_then(|s| s.get_last_edition()) {
                Ok(None) => None,
                // Do not publish draft editions in non-draft series!
                Ok(Some(latest)) if latest.is_draft() => None,
                Ok(Some(latest)) => {
                    let cipher_key = latest.public_key().hash();
                    let rand = Hash::rand();
                    let cipher = TransferCipher::new(&cipher_key, &rand);

                    Some(EditionResponse {
                        rand,
                        series: cipher.encrypt_opaque(&latest),
                    })
                }
                Err(err) => {
                    log::error!("error resolving edition for {latest:?}: {err}");
                    None
                }
            }
        } else {
            None
        };

        if let Some(response) = maybe_response.as_ref() {
            log::info!("Edition found: {response:?}");
        } else {
            log::info!("Edition not found");
        }

        maybe_response.into_iter().collect()
    }

    async fn announce_edition(self, _: context::Context, announcement: Arc<EditionAnnouncement>) {
        log::info!("Got announcement from hub");
        match SubscriptionRef::find(&announcement.key_riddle) {
            Err(err) => log::error!("error processing {announcement:?}: {err}"),
            Ok(None) => {
                log::info!("No subscription found for announcement");
            }
            Ok(Some(subscription)) => {
                log::info!("Found {subscription} for announcement");

                let cipher =
                    TransferCipher::new(&subscription.public_key.hash(), &announcement.rand);

                let try_refresh = async move {
                    let edition: Edition = announcement.edition.clone().decrypt_with(&cipher)?;

                    if !edition.is_valid() {
                        log::warn!("an invalid edition was announced: {:?}", edition);
                        return Ok(());
                    }

                    if subscription.must_refresh()? {
                        edition.refresh().await
                    } else {
                        Ok(())
                    }
                };

                tokio::spawn(async move {
                    // Sleep a random amount so as not for everybody to ask for the same items at
                    // the same time.
                    tokio::time::sleep(std::time::Duration::from_secs_f32(rand::random())).await;
                    if let Err(err) = try_refresh.await {
                        log::warn!("{}", err);
                    }
                });
            }
        }
    }

    async fn get_identity(
        self,
        _ctx: context::Context,
        request: Arc<IdentityRequest>,
    ) -> Vec<IdentityResponse> {
        log::info!("Got identity request from hub: {request:?}");

        let maybe_response = match Identity::find(&request.identity_riddle) {
            Err(err) => {
                log::error!("Error while processing {request:?}: {err}");
                None
            }
            Ok(None) => {
                log::info!("No identity found for request");
                None
            }
            Ok(Some(identity)) => {
                let cipher_key = identity.identity().hash();
                let rand = Hash::rand();
                let cipher = TransferCipher::new(&cipher_key, &rand);

                log::info!("Found identity {}", identity.identity().handle());

                Some(IdentityResponse {
                    rand,
                    identity: cipher.encrypt_opaque(&identity),
                })
            }
        };

        maybe_response.into_iter().collect()
    }

    async fn announce_identity(
        self,
        _ctx: context::Context,
        _announcement: Arc<IdentityAnnouncement>,
    ) {
        // This is a no-op, by now.
    }
}
