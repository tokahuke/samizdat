use crate::access_token;

pub async fn ls(collection: String) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "{}/_collections/{}/_list",
            crate::server(),
            collection
        ))
        .header("Authorization", format!("Bearer {}", access_token()))
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
