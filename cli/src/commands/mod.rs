pub mod collection;
pub mod series;

use askama::Template;
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

use samizdat_common::{Hash, Key, PrivateKey, Signed};

use crate::html::maybe_proxy_page;
use crate::{Manifest, PrivateManifest};

fn show_table<T: Tabled>(t: impl IntoIterator<Item = T>) {
    println!("{}", Table::new(t).with(tabled::Style::github_markdown()))
}

pub async fn upload(
    path: &Path,
    content_type: String,
    bookmark: bool,
    is_draft: bool,
) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "{}/_objects?bookmark={}&is-draft={}",
            crate::server(),
            bookmark,
            is_draft,
        ))
        .header("Content-Type", content_type)
        .body(fs::read(path)?)
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Object hash: {}", response.text().await?);

    Ok(())
}

pub async fn init() -> Result<(), crate::Error> {
    let pwd = env::current_dir()?;
    let name = pwd.iter().last().expect("not empty").to_string_lossy();
    let debug_name = format!("{}-debug", name);

    #[derive(Serialize)]
    struct Request<'a> {
        series_owner_name: &'a str,
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/_seriesowners", crate::server()))
        .json(&Request {
            series_owner_name: &name,
        })
        .send()
        .await?;
    let response_debug = client
        .post(format!("{}/_seriesowners", crate::server()))
        .json(&Request {
            series_owner_name: &debug_name,
        })
        .send()
        .await?;

    if response.status().is_success() {
        #[derive(Deserialize)]
        struct Payoad {
            //name: String,
            keypair: ed25519_dalek::Keypair,
            default_ttl: Duration,
        }

        let text = response.text().await?;
        let payload: Payoad = serde_json::from_str(&text)?;
        let text_debug = response_debug.text().await?;
        let payload_debug: Payoad = serde_json::from_str(&text_debug)?;

        let rendered = crate::manifest::ManifestTemplate {
            name: &name,
            public_key: &Key::from(payload.keypair.public).to_string(),
            ttl: &humantime::format_duration(payload.default_ttl).to_string(),
            debug_name: &debug_name,
            public_key_debug: &Key::from(payload_debug.keypair.public).to_string(),
        }
        .render()
        .expect("can render");

        let rendered_private = crate::manifest::PrivateManifestTemplate {
            private_key: &PrivateKey::from(payload.keypair.secret).to_string(),
            private_key_debug: &PrivateKey::from(payload_debug.keypair.secret).to_string(),
        }
        .render()
        .expect("can render");

        fs::write("./Samizdat.toml", rendered)?;
        fs::write("./.Samizdat.priv", rendered_private)?;
    } else {
        println!("Bad status: {}", response.status());
    }

    Ok(())
}

pub async fn commit(ttl: &Option<String>, is_release: bool) -> Result<(), crate::Error> {
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
    manifest.run(is_release)?;

    let mut all_files = vec![];
    walk(base, &mut all_files)?;

    log::debug!("committing: {:#?}", all_files);

    let client = reqwest::Client::new();
    let client = &client; // TODO: awaiting new Rust edition.

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
                    !is_release
                ))
                .header("Content-Type", content_type)
                .body(maybe_proxy_page(path, &fs::read(&path)?).into_owned())
                .send()
                .await?;
            let hash = if response.status().is_success() {
                response.text().await?
            } else {
                return Err("??".into()) as Result<_, crate::Error>;
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

    #[derive(Serialize)]
    struct Request {
        hashes: Vec<(String, String)>,
        is_draft: bool,
    }

    let response = client
        .post(format!("{}/_collections", crate::server()))
        .json(&Request {
            hashes,
            is_draft: !is_release,
        })
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    let collection = response.text().await?;

    let series = if is_release {
        manifest.series.name
    } else {
        manifest.debug.name
    };
    let ttl = ttl.clone().or(if is_release {
        manifest.series.ttl
    } else {
        manifest.debug.ttl
    });

    #[derive(Serialize)]
    struct CollectionRequest {
        collection: String,
        ttl: Option<String>,
    }

    let response = client
        .post(format!(
            "{}/_seriesowners/{}/collections",
            crate::server(),
            series,
        ))
        .json(&CollectionRequest { collection, ttl })
        .send()
        .await?;

    if response.status().is_client_error() {
        let status = response.status();
        let text = response.text().await?;
        println!("Status: {}", status);
        println!("Response: {}", text);

        return Err(crate::Error::Message(format!(
            "series {} does not exist",
            series
        )));
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct CollectionRef {
        pub hash: Hash,
    }

    #[derive(Debug, Deserialize)]
    struct SeriesItemContent {
        collection: CollectionRef,
        timestamp: chrono::DateTime<chrono::Utc>,
        ttl: Duration,
    }

    #[derive(Debug, Deserialize)]
    pub struct SeriesItem {
        signed: Signed<SeriesItemContent>,
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

    let text = response.text().await?;
    let item: SeriesItem = serde_json::from_str(&text).map_err(|err| {
        println!("bad json: {}", text);
        err
    })?;

    show_table([Row {
        series: series.to_owned(),
        // public_key: item.public_key,
        collection: item.signed.collection.hash,
        timestamp: item.signed.timestamp,
        ttl: format!("{:?}", item.signed.ttl),
    }]);

    Ok(())
}

pub async fn watch(ttl: &Option<String>) -> Result<(), crate::Error> {
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
    if let Err(err) = commit(ttl, false).await {
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
            if let Err(err) = commit(ttl, false).await {
                println!("Error while rebuilding: {}", err);
            }

            last_exec = Instant::now();
        }
    }

    Ok(())
}

pub async fn import() -> Result<(), crate::Error> {
    let manifest = Manifest::find()?;
    let private_manifest = PrivateManifest::find()?;

    #[derive(Serialize)]
    struct KeyPair<'a> {
        public_key: &'a str,
        private_key: &'a str,
    }

    #[derive(Serialize)]
    struct Request<'a> {
        series_owner_name: &'a str,
        keypair: KeyPair<'a>,
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/_seriesowners", crate::server()))
        .json(&Request {
            series_owner_name: &manifest.series.name,
            keypair: KeyPair {
                public_key: &manifest.series.public_key,
                private_key: &private_manifest.private_key,
            },
        })
        .send()
        .await?;

    let _debug_response = client
        .post(format!("{}/_seriesowners", crate::server()))
        .json(&Request {
            series_owner_name: &manifest.debug.name,
            keypair: KeyPair {
                public_key: &manifest.debug.public_key,
                private_key: &private_manifest.private_key_debug,
            },
        })
        .send()
        .await?;

    println!("Status: {}", response.status());
    // println!("Response: {}", response.text().await?);

    Ok(())
}
