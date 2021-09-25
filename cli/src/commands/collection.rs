pub async fn ls(collection: &str) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://localhost:4510/_collections/{}/_list",
            collection
        ))
        .send()
        .await?;

    log::info!("Status: {}", response.status());

    if response.status().is_success() {
        let paths = response.json::<Vec<String>>().await?;
        println!("{}/", collection);
        crate::util::print_paths(&paths, '/');
    } else {
        println!("{}", response.text().await?);
    }

    Ok(())
}
