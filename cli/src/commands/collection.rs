use crate::api;

pub async fn ls(collection: String) -> Result<(), crate::Error> {
    let paths: Vec<String> = api::get(format!("/_collections/{}/_list", collection)).await??;

    println!("{}/", collection);
    crate::util::print_paths(&paths, '/');

    Ok(())
}
