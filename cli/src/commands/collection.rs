use crate::api;

pub async fn ls(collection: String) -> Result<(), anyhow::Error> {
    let paths = api::get_collection_list(&collection).await?;

    println!("{}/", collection);
    crate::util::print_paths(&paths, '/');

    Ok(())
}
