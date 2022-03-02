pub mod auth;
pub mod collection;
pub mod identity;
pub mod series;
pub mod subscription;

use anyhow::Context;
use futures::prelude::*;
use futures::stream;
use notify::{RecursiveMode, Watcher};
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{env, fs, io};
use tabled::{Table, Tabled};
use tokio::sync::mpsc;

use samizdat_common::{Hash, PrivateKey};

use crate::api;
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
) -> Result<(), anyhow::Error> {
    let hash = api::post_object(fs::read(path)?, &content_type, bookmark, is_draft).await?;
    println!("Object hash: {hash}");

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

    println!(
        "NOTE: Your private key for this project is \n\n\t{}
        \n\nStore it somewhere safe! (you were warned)",
        private_key
    );

    Ok(())
}

pub async fn import(private_key: Option<String>) -> Result<(), anyhow::Error> {
    let manifest = Manifest::find_opt()
        .context("failed to find `Samizdat.toml`")?
        .ok_or_else(|| anyhow::anyhow!("`Samizdat.toml` does not exist"))?;
    let private_manifest_opt =
        PrivateManifest::find_opt().context("failed to find `.Samizdat.priv`")?;
    let private_manifest = if let Some(private_manifest) = private_manifest_opt {
        private_manifest
    } else {
        PrivateManifest::create(
            &manifest.debug.name,
            private_key
                .map(|pk| pk.parse::<PrivateKey>())
                .transpose()?
                .as_ref(),
        )
        .await
        .context("failed to create `.Samizdat.priv`")?
    };

    // Import debug series owner.
    api::post_series_owner(api::PostSeriesOwnerRequest {
        series_owner_name: &manifest.debug.name,
        keypair: Some(api::Keypair {
            private_key: private_manifest.private_key_debug,
            public_key: private_manifest.public_key_debug,
        }),
        is_draft: false,
    })
    .await
    .context("failed to import series keypair")?;

    // Import series owners if its private key present in the private manifest.
    if let Some(private_key) = private_manifest.private_key {
        api::post_series_owner(api::PostSeriesOwnerRequest {
            series_owner_name: &manifest.series.name,
            keypair: Some(api::Keypair {
                private_key,
                public_key: manifest.series.public_key,
            }),
            is_draft: false,
        })
        .await
        .context("failed to import series keypair")?;
    }

    Ok(())
}

pub async fn commit(
    ttl: &Option<String>,
    is_release: bool,
    no_announce: bool,
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
        names.sort_unstable();
        names.dedup();

        names
    }

    let manifest =
        Manifest::find_opt()?.ok_or_else(|| anyhow::anyhow!("`Samizdat.toml` does not exst"))?;
    let private_manifest = PrivateManifest::find_opt()?.ok_or_else(|| {
        anyhow::anyhow!("Private manifest `.Samizdat.priv` not found. Hint: run `samizdat import`.")
    })?;

    if is_release && private_manifest.private_key.is_none() {
        anyhow::bail!(
            "Cannot run release mode without a private key. Hint: put your private key \
            in `.Samizdat.priv` and then run `samizdat import`."
        )
    }

    let base = &manifest.build.base;
    manifest.run_build(is_release)?;

    let mut all_files = vec![];
    walk(base, &mut all_files)?;

    log::debug!("committing: {:#?}", all_files);

    let hashes = stream::iter(&all_files)
        .map(|path| async move {
            log::info!("Creating object for {:?}", path);
            let content_type = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();

            let hash = api::post_object(
                maybe_proxy_page(path, &fs::read(&path)?).into_owned(),
                &content_type,
                is_release,
                !is_release,
            )
            .await?;

            let names = names_from_path(path, base);

            Ok((names, hash)) as Result<(Vec<String>, String), anyhow::Error>
        })
        .buffer_unordered(all_files.len())
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flat_map(|(names, hash)| names.into_iter().map(move |name| (name, hash.clone())))
        .collect::<Vec<_>>();

    log::debug!("hashes: {:#?}", hashes);

    let collection = api::post_collection(api::PostCollectionRequest {
        hashes: &hashes,
        is_draft: !is_release,
    })
    .await?;

    let series_name = if is_release {
        manifest.series.name
    } else {
        manifest.debug.name
    };
    let ttl = ttl.clone().or(if is_release {
        manifest.series.ttl
    } else {
        None
    });

    let edition = api::post_edition(
        &series_name,
        api::PostEditionRequest {
            collection: &collection,
            ttl: ttl.as_deref(),
            no_announce,
        },
    )
    .await?;

    #[derive(Tabled)]
    struct Row {
        series: String,
        // public_key: Key,
        collection: Hash,
        timestamp: chrono::DateTime<chrono::Utc>,
        ttl: String,
    }

    show_table([Row {
        series: series_name.to_owned(),
        // public_key: item.public_key,
        collection: edition.signed.collection.hash,
        timestamp: edition.signed.timestamp,
        ttl: format!("{:?}", edition.signed.ttl),
    }]);

    Ok(())
}

pub async fn watch(ttl: &Option<String>) -> Result<(), anyhow::Error> {
    /// Minimum time you have to wait to trigger rebuild.
    const MIN_WAIT: Duration = Duration::from_secs(1);

    let manifest =
        Manifest::find_opt()?.ok_or_else(|| anyhow::anyhow!("`Samizdat.toml` does not exist"))?;
    let private_manifest = PrivateManifest::find_opt()?.ok_or_else(|| {
        anyhow::anyhow!("Private manifest `.Samizdat.priv` not found. Hint: run `samizdat import`.")
    })?;

    // Ignore the output folder.
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
                log::error!("Notify error: {err}");
            }
        }
    })?;

    log::info!("Installing watcher");

    watcher.watch(Path::new("."), RecursiveMode::Recursive)?;

    // Print watch banner:
    const MARKER: &str = "\u{001b}[1m\u{001b}[31m*\u{001b}[0m";
    println!();
    println!(
        "{MARKER} Publishing series at \u{001b}[1mhttp://localhost:{}/_series/{}\u{001b}[0m",
        crate::cli::cli().port,
        private_manifest.public_key_debug
    );
    println!();

    log::info!("Starting rebuild loop");

    // Run the commit for the first time.
    if let Err(err) = commit(ttl, false, true).await {
        println!("Error while rebuilding: {err:?}");
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
                println!("Error while rebuilding: {err:?}");
            }

            last_exec = Instant::now();
        }
    }

    Ok(())
}
