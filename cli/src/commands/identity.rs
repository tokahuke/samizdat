use crate::api::{get_ethereum_provider, put_ethereum_provider};

pub async fn set_provider(endpoint: &str) -> Result<(), anyhow::Error> {
    put_ethereum_provider(endpoint.to_owned()).await?;
    Ok(())
}

pub async fn get_provider() -> Result<(), anyhow::Error> {
    let endpoint = get_ethereum_provider().await?.endpoint;
    print!("{endpoint}");
    Ok(())
}
