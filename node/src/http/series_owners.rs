//! Series API.

use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{SubsecRound, Utc};
use futures::FutureExt;
use serde_derive::Deserialize;

use crate::access::AccessRight;
use crate::http::ApiResponse;
use crate::models::{
    CollectionRef, Droppable, EditionKind, Inventory, ItemPathBuf, ObjectRef, SeriesOwner,
};
use crate::{hubs, security_scope};

/// The entrypoint of the series API.
pub fn api() -> Router {
    #[derive(Deserialize)]
    struct Keypair {
        private_key: String,
    }

    #[derive(Deserialize)]
    struct PostSeriesOwnerRequest {
        series_owner_name: String,
        #[serde(default)]
        keypair: Option<Keypair>,
        #[serde(default)]
        is_draft: bool,
    }

    #[derive(Deserialize)]
    struct PostEditionRequest {
        kind: EditionKind,
        #[serde(default)]
        #[serde(with = "humantime_serde")]
        ttl: Option<std::time::Duration>,
        #[serde(default)]
        no_announce: bool,
        #[serde(default)]
        is_draft: bool,
        hashes: Vec<(String, String)>,
    }

    Router::new()
        .route(
            // Creates a new series owner, i.e., a public-private keypair that allows one to push new
            // collections to a series.
            "/",
            post(|Json(request): Json<PostSeriesOwnerRequest>| {
                async move {
                    if let Some(Keypair { private_key }) = request.keypair {
                        SeriesOwner::import(
                            &request.series_owner_name,
                            private_key.parse()?,
                            Duration::from_secs(3_600),
                            request.is_draft,
                        )
                    } else {
                        SeriesOwner::create(
                            &request.series_owner_name,
                            Duration::from_secs(3_600),
                            request.is_draft,
                        )
                    }
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSeries)),
        )
        .route(
            // Gets information associates with a series owner
            "/:series_owner_name",
            get(|Path(series_owner_name): Path<String>| {
                async move {
                    let maybe_owner = SeriesOwner::get(&series_owner_name)?;
                    Ok(maybe_owner)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSeries)),
        )
        .route(
            // Removes a series owner
            "/:series_owner_name",
            delete(|Path(series_owner_name): Path<String>| {
                async move {
                    let maybe_owner = SeriesOwner::get(&series_owner_name)?;
                    let existed = maybe_owner
                        .map(|owner| owner.drop_if_exists())
                        .transpose()?
                        .is_some();
                    Ok(existed)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSeries)),
        )
        .route(
            "/",
            get(|| async move { SeriesOwner::get_all() }.map(ApiResponse))
                .layer(security_scope!(AccessRight::ManageSeries)),
        )
        .route(
            // Pushes a new collection to the series owner, creating a new edition.
            "/:series_owner_name/editions",
            post(
                |Path(series_owner_name): Path<String>, Json(request): Json<PostEditionRequest>| {
                    async move {
                        let Some(series_owner) = SeriesOwner::get(&series_owner_name)? else {
                            return Err(crate::Error::Message(format!(
                                "Series owner {} not found",
                                series_owner_name
                            )));
                        };

                        // Set edition timestamp:
                        let timestamp = Utc::now().trunc_subsecs(0);

                        // Build collection:
                        let collection = tokio::task::spawn_blocking(move || {
                            // Decode hashes:
                            let hashes = request
                                .hashes
                                .into_iter()
                                .map(|(name, hash)| {
                                    Ok((ItemPathBuf::from(name), ObjectRef::new(hash.parse()?)))
                                })
                                .collect::<Result<Vec<_>, crate::Error>>()?;

                            // Create edition inventory:
                            let inventory_path = match request.kind {
                                EditionKind::Base => ItemPathBuf::from("_inventory"),
                                EditionKind::Layer => {
                                    ItemPathBuf::from(format!("_changelogs/{timestamp}"))
                                }
                            };
                            let hashes_with_inventory = Inventory::insert_into_list(
                                request.is_draft,
                                inventory_path,
                                hashes,
                            )?;

                            // Create collection:
                            CollectionRef::build(request.is_draft, hashes_with_inventory)
                        })
                        .await
                        .expect("Collection build task failed")?;

                        // Create edition:
                        let edition = series_owner.advance(
                            collection,
                            timestamp,
                            request.ttl,
                            request.kind,
                        )?;

                        if !request.no_announce {
                            let announcement = edition.announcement();
                            tokio::spawn({
                                let edition = edition.clone();
                                async move {
                                    tracing::info!("Announcing edition {edition:?}");
                                    hubs().announce_edition(&announcement).await
                                }
                            });
                        }

                        Ok(edition)
                    }
                    .map(ApiResponse)
                },
            )
            .layer(
                tower::ServiceBuilder::new()
                    .layer(security_scope!(AccessRight::ManageSeries))
                    .layer(DefaultBodyLimit::disable()),
            ),
        )
}
