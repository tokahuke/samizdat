//! RPC implementation for the Node. This RPC is called by the hubs to trigger object resolution.

use futures::prelude::*;
use samizdat_common::db::readonly_tx;
use std::sync::Arc;
use tarpc::context;

use samizdat_common::address::{ChannelAddr, ChannelId};
use samizdat_common::cipher::TransferCipher;
use samizdat_common::keyed_channel::KeyedChannel;
use samizdat_common::rpc::*;
use samizdat_common::{Hash, Riddle};

use crate::cli;
use crate::models::{CollectionItem, Edition, ObjectRef, SeriesRef, SubscriptionRef};
use crate::system::transport::channel_manager;

use super::file_transfer;

#[derive(Clone)]
pub struct NodeServer {
    pub candidate_channels: KeyedChannel<Candidate>,
}

impl NodeServer {
    async fn resolve_object(self, resolution: Arc<Resolution>) -> ResolutionResponse {
        tracing::info!("got object {resolution:?}");

        let content_riddle = if let Some(content_riddle) = resolution.content_riddles.first() {
            content_riddle
        } else {
            return ResolutionResponse::EmptyResolution;
        };

        if resolution.hint.len() < cli().min_hint_size as usize {
            tracing::warn!(
                "Resolution hint length is {}, smaller than the minimum of {}. Ignoring...",
                resolution.hint.len(),
                cli().min_hint_size
            );
            return ResolutionResponse::NotFound;
        }

        let object = match readonly_tx(|tx| ObjectRef::find(tx, content_riddle, &resolution.hint)) {
            Ok(Some(object)) if !readonly_tx(|tx| object.is_draft(tx)).unwrap_or(true) => object,
            Ok(Some(_)) => {
                tracing::info!("Hash found but object is draft");
                return ResolutionResponse::NotFound;
            }
            Ok(None) => {
                tracing::info!("Hash not found for resolution");
                return ResolutionResponse::NotFound;
            }
            Err(err) => {
                tracing::error!("Error while looking for object {resolution:?}: {err}");
                return ResolutionResponse::NotFound;
            }
        };

        let hash = *object.hash();

        tracing::info!("Found hash {}", hash);
        let Some(peer_addr) = resolution
            .location_message_riddle
            .resolve::<ChannelAddr>(&hash)
        else {
            tracing::warn!("Failed to resolve message riddle after resolving content riddle");
            return ResolutionResponse::NotFound;
        };

        tracing::info!("Found peer at {peer_addr}");

        tokio::spawn(
            async move {
                tracing::info!("Starting task to transfer object {} to {}", hash, peer_addr);
                let (sender, receiver) = channel_manager::initiate(peer_addr).await?;
                file_transfer::send_object(sender, receiver, &object).await
            }
            .map(move |outcome| {
                outcome.map_err(|err| {
                    tracing::error!("Failed to send {} to {}: {}", hash, peer_addr, err)
                })
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
        tracing::info!("got item {:?}", resolution);

        let Some(content_riddle) = resolution.content_riddles.first() else {
            return ResolutionResponse::EmptyResolution;
        };

        if resolution.hint.len() < cli().min_hint_size as usize {
            tracing::warn!(
                "Resolution hint length is {}, smaller than the minimum of {}. Ignoring...",
                resolution.hint.len(),
                cli().min_hint_size
            );
            return ResolutionResponse::NotFound;
        }

        let item =
            match readonly_tx(|tx| CollectionItem::find(tx, content_riddle, &resolution.hint)) {
                Ok(Some(item)) if !item.is_draft => item,
                Ok(Some(_)) => {
                    tracing::info!("hash found, but item is draft");
                    return ResolutionResponse::NotFound;
                }
                Ok(None) => {
                    tracing::info!("hash not found for resolution");
                    return ResolutionResponse::NotFound;
                }
                Err(e) => {
                    tracing::error!("error looking for hash: {}", e);
                    return ResolutionResponse::NotFound;
                }
            };

        // Code smell?
        let hash = item.locator().hash();

        tracing::info!("found hash {}", hash);
        let Some(peer_addr) = resolution
            .location_message_riddle
            .resolve::<ChannelAddr>(&hash)
        else {
            tracing::warn!("failed to resolve message riddle after resolving content riddle");
            return ResolutionResponse::NotFound;
        };

        tracing::info!("found peer at {}", peer_addr);

        tokio::spawn(
            async move {
                let (sender, receiver) = channel_manager::initiate(peer_addr).await?;
                file_transfer::send_item(sender, receiver, item).await
            }
            .map(move |outcome| {
                outcome.map_err(|err| {
                    tracing::error!("failed to send {} to {}: {}", hash, peer_addr, err)
                })
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

impl Node for NodeServer {
    async fn config(self, _: context::Context) -> NodeConfig {
        NodeConfig {
            max_queries: cli().max_queries_per_hub,
            max_query_rate: cli().max_query_rate_per_hub,
        }
    }

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
        tracing::info!("got {latest:?}");

        if latest.hint.len() < cli().min_hint_size as usize {
            tracing::warn!(
                "Edition request hint length is {}, smaller than the minimum of {}. Ignoring...",
                latest.hint.len(),
                cli().min_hint_size
            );
            return vec![];
        }

        let maybe_response = if let Some(series) =
            readonly_tx(|tx| SeriesRef::find(tx, &latest.key_riddle, &latest.hint)).transpose()
        {
            match series.map(|s| readonly_tx(|tx| s.get_last_edition(tx))) {
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
                    tracing::error!("error resolving edition for {latest:?}: {err}");
                    None
                }
            }
        } else {
            None
        };

        if let Some(response) = maybe_response.as_ref() {
            tracing::info!("Edition found: {response:?}");
        } else {
            tracing::info!("Edition not found");
        }

        maybe_response.into_iter().collect()
    }

    async fn announce_edition(self, _: context::Context, announcement: Arc<EditionAnnouncement>) {
        tracing::info!("Got announcement from hub");

        if announcement.hint.len() < cli().min_hint_size as usize {
            tracing::warn!(
                "Announcement hint length is {}, smaller than the minimum of {}. Ignoring...",
                announcement.hint.len(),
                cli().min_hint_size
            );
            return;
        }

        match readonly_tx(|tx| {
            SubscriptionRef::find(tx, &announcement.key_riddle, &announcement.hint)
        }) {
            Err(err) => tracing::error!("error processing {announcement:?}: {err}"),
            Ok(None) => {
                tracing::info!("No subscription found for announcement");
            }
            Ok(Some(subscription)) => {
                tracing::info!("Found {subscription} for announcement");

                let cipher =
                    TransferCipher::new(&subscription.public_key.hash(), &announcement.rand);

                let try_refresh = async move {
                    let edition: Edition = announcement.edition.clone().decrypt_with(&cipher)?;

                    if !edition.is_valid() {
                        tracing::warn!("an invalid edition was announced: {:?}", edition);
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
                        tracing::warn!("{}", err);
                    }
                });
            }
        }
    }
}
