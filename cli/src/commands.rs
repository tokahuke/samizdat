use futures::prelude::*;
use futures::stream;
use std::path::PathBuf;
use std::{fs, io};

pub async fn upload(path: &PathBuf, content_type: String) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .post("http://localhost:4510/_objects")
        .header("Content-Type", content_type)
        .body(fs::read(path)?)
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Object hash: {}", response.text().await?);

    Ok(())
}

pub async fn init() -> Result<(), crate::Error> {
    todo!()
}

pub async fn commit(base: &PathBuf) -> Result<(), crate::Error> {
    // Oh, generators would be so nice now...
    fn walk(path: &PathBuf, files: &mut Vec<PathBuf>) -> io::Result<()> {
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

    fn names_from_path(path: &PathBuf, base: &PathBuf) -> Vec<String> {
        let suffixed = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let without_index = suffixed.trim_end_matches("index.html").to_owned();
        let without_slash = without_index.trim_end_matches("/").to_owned();

        let mut names = vec![suffixed, without_index, without_slash];
        names.sort();
        names.dedup();

        names
    }

    let mut all_files = vec![];
    walk(base, &mut all_files)?;

    log::debug!("committing: {:#?}", all_files);

    let client = reqwest::Client::new();
    let client = &client; // TODO: awaiting new Rust edition.

    let hashes = stream::iter(&all_files)
        .map(|path| async move {
            println!("Creating object for {:?}", path);
            let content_type = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();
            let response = client
                .post("http://localhost:4510/_objects")
                .header("Content-Type", content_type)
                .body(fs::read(&path)?)
                .send()
                .await?;
            let hash = if response.status().is_success() {
                response.text().await?
            } else {
                return Err(crate::Error::Message(format!("")));
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

    let response = client
        .post("http://localhost:4510/_collections")
        .json(&hashes)
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Collection hash: {}", response.text().await?);

    Ok(())
}
