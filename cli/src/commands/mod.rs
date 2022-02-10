pub mod auth;
pub mod collection;
pub mod series;
pub mod subscription;

use anyhow::Context;
use futures::prelude::*;
use futures::stream;
use notify::{RecursiveMode, Watcher};
use serde_derive::{Deserialize, Serialize};
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{env, fs, io};
use tabled::{Table, Tabled};
use tokio::sync::mpsc;

use samizdat_common::{Hash, Signed};

use crate::api::ApiError;
use crate::html::maybe_proxy_page;
use crate::{access_token, api};
use crate::{Manifest, PrivateManifest};

fn show_table<T: Tabled>(t: impl IntoIterator<Item = T>) {
    println!("{}", Table::new(t).with(tabled::Style::github_markdown()))
}

pub async fn upload(
    path: &Path,
    content_type: String,
    bookmark: bool,
    is_draft: bool,
) -> Result<(), anyhow::Error> {
    let response = api::CLIENT
        .post(format!(
            "{}/_objects?bookmark={}&is-draft={}",
            crate::server(),
            bookmark,
            is_draft,
        ))
        .header("Content-Type", content_type)
        .header("Authorization", format!("Bearer {}", access_token()))
        .body(fs::read(path)?)
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    let returned: Result<String, ApiError> = response.json().await?;
    println!("Object hash: {}", returned?);

    Ok(())
}

pub async fn init() -> Result<(), anyhow::Error> {
    let pwd = env::current_dir()?;
    let name = pwd.iter().last().expect("not empty").to_string_lossy();

    let (manifest, private_key) = Manifest::create(&name)
        .await
        .context("failed to create `Manifest.toml`")?;
    PrivateManifest::create(&manifest.debug.name, Some(&private_key))
        .await
        .context("failed to create `.Samizdat.priv`")?;

    Ok(())
}

pub async fn import() -> Result<(), anyhow::Error> {
    let manifest = Manifest::find().context("failed to find `Samizdat.toml`")?;
    let private_manifest_opt =
        PrivateManifest::find_opt().context("failed to find `.Samizdat.priv`")?;
    let private_manifest = if let Some(private_manifest) = private_manifest_opt {
        private_manifest
    } else {
        PrivateManifest::create(&manifest.debug.name, None)
            .await
            .context("failed to create `.Samizdat.priv`")?
    };

    // Import series owners if it private key present in the private manifest.
    if let Some(private_key) = private_manifest.private_key {
        #[derive(Debug, Serialize)]
        struct Keypair {
            public_key: String,
            private_key: String,
        }

        #[derive(Debug, Serialize)]
        struct Request {
            series_owner_name: String,
            keypair: Keypair,
            is_draft: bool,
        }

        api::post(
            "/_seriesowners",
            Request {
                series_owner_name: manifest.series.name,
                keypair: Keypair {
                    private_key,
                    public_key: manifest.series.public_key,
                },
                is_draft: false,
            },
        )
        .await
        .context("failed to import series keypair")?;
    }

    Ok(())
}

pub async fn commit(
    ttl: &Option<String>,
    is_release: bool,
    no_annouce: bool,
) -> Result<(), anyhow::Error> {
    // Oh, generators would be so nice now...
    fn walk(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let subpath = entry.path();

            if subpath.is_dir() {
                walk(&subpath, files)?;
            } else {
                // file or symlink.
                files.push(subpath);
            }
        }

        Ok(())
    }

    fn names_from_path(path: &Path, base: &Path) -> Vec<String> {
        let suffixed = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let without_index = suffixed.trim_end_matches("index.html").to_owned();
        let without_slash = without_index.trim_end_matches('/').to_owned();

        let mut names = vec![suffixed, without_index, without_slash];
        names.sort();
        names.dedup();

        names
    }

    let manifest = Manifest::find()?;
    let base = &manifest.build.base;
    manifest.run_build(is_release)?;

    let mut all_files = vec![];
    walk(base, &mut all_files)?;

    log::debug!("committing: {:#?}", all_files);

    let client = reqwest::Client::new();
    let client = &client;

    let hashes = stream::iter(&all_files)
        .map(|path| async move {
            log::info!("Creating object for {:?}", path);
            let content_type = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();
            let response = client
                .post(format!(
                    "{}/_objects?is_draft={}",
                    crate::server(),
                    !is_release,
                    //fs::metadata(&path)?.created().expect("created time exists"),
                ))
                .header("Content-Type", content_type)
                .header("Authorization", format!("Bearer {}", access_token()))
                .body(maybe_proxy_page(path, &fs::read(&path)?).into_owned())
                .send()
                .await?;
            let hash = if response.status().is_success() {
                response.json::<Result<String, ApiError>>().await??
            } else {
                return Err(anyhow::anyhow!("??")) as Result<_, anyhow::Error>;
            };
            let names = names_from_path(path, base);

            Ok((names, hash))
        })
        .buffer_unordered(all_files.len())
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flat_map(|(names, hash)| names.into_iter().map(move |name| (name, hash.clone())))
        .collect::<Vec<_>>();

    log::debug!("hashes: {:#?}", hashes);

    #[derive(Debug, Serialize)]
    struct Request {
        hashes: Vec<(String, String)>,
        is_draft: bool,
    }

    let collection = api::post(
        "/_collections",
        Request {
            hashes,
            is_draft: !is_release,
        },
    )
    .await?;

    let series = if is_release {
        manifest.series.name
    } else {
        manifest.debug.name
    };
    let ttl = ttl.clone().or(if is_release {
        manifest.series.ttl
    } else {
        None
    });

    #[derive(Debug, Serialize)]
    struct CollectionRequest {
        collection: String,
        ttl: Option<String>,
        no_annouce: bool,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct CollectionRef {
        pub hash: Hash,
    }

    #[derive(Debug, Deserialize)]
    struct EditionContent {
        collection: CollectionRef,
        timestamp: chrono::DateTime<chrono::Utc>,
        #[serde(with = "humantime_serde")]
        ttl: Duration,
    }

    #[derive(Debug, Deserialize)]
    pub struct Edition {
        signed: Signed<EditionContent>,
        //public_key: Key,
        //freshness: chrono::DateTime<chrono::Utc>,
    }

    #[derive(Tabled)]
    struct Row {
        series: String,
        // public_key: Key,
        collection: Hash,
        timestamp: chrono::DateTime<chrono::Utc>,
        ttl: String,
    }

    let edition: Result<Edition, ApiError> = api::post(
        format!("/_seriesowners/{}/editions", series,),
        CollectionRequest {
            collection,
            ttl,
            no_annouce,
        },
    )
    .await?;

    if let Ok(edition) = edition {
        show_table([Row {
            series: series.to_owned(),
            // public_key: item.public_key,
            collection: edition.signed.collection.hash,
            timestamp: edition.signed.timestamp,
            ttl: format!("{:?}", edition.signed.ttl),
        }]);

        Ok(())
    } else {
        Err(anyhow::anyhow!("series {} does not exist", series))
    }
}

pub async fn watch(ttl: &Option<String>) -> Result<(), anyhow::Error> {
    /// Minimum time you ave to wait to trigger rebuild.
    const MIN_WAIT: Duration = Duration::from_secs(1);

    // Ignore the output folder.
    let manifest = Manifest::find()?;
    let base = if manifest.build.base.is_absolute() {
        manifest.build.base.clone()
    } else {
        std::env::current_dir()
            .expect("current dir exists")
            .join(&manifest.build.base)
    };

    // Spawn file watcher.
    let (send, mut recv) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        log::info!("Starting to listen for events");
        match event {
            Ok(event) => {
                send.send(event).ok();
            }
            Err(err) => {
                log::error!("Notify error: {}", err);
            }
        }
    })?;

    log::info!("Installing watcher");

    watcher.watch(Path::new("."), RecursiveMode::Recursive)?;

    log::info!("Starting rebuild loop");

    // Run the commit for the first time.
    if let Err(err) = commit(ttl, false, true).await {
        println!("Error while rebuilding: {}", err);
    }

    // Last time the commit was triggered.
    let mut last_exec = Instant::now();

    // The commit loop.
    while let Some(event) = recv.recv().await {
        let now = Instant::now();
        let watched_files_changed = event.paths.iter().any(|path| !path.starts_with(&base));

        if watched_files_changed && now > last_exec + MIN_WAIT {
            log::info!("Rebuild triggered");
            if let Err(err) = commit(ttl, false, true).await {
                println!("Error while rebuilding: {}", err);
            }

            last_exec = Instant::now();
        }
    }

    Ok(())
}
