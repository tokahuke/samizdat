//! Collection command implementations for the Samizdat CLI.
use crate::api;

/// Lists the contents of a collection.
///
/// # Arguments
/// * `collection` - The hash of the collection to list
pub async fn ls(collection: String) -> Result<(), anyhow::Error> {
    let paths = api::get_collection_list(&collection).await?;

    println!("{}/", collection);
    crate::util::print_paths(&paths, '/');

    Ok(())
}
