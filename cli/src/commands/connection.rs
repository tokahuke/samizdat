use tabled::Tabled;

use crate::api::{self};

use super::show_table;

pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_connections().await?;

    #[derive(Tabled)]
    struct Row {
        name: String,
        status: String,
        addr: String,
    }

    show_table(
        response
            .into_iter()
            .map(|conn| Row {
                name: conn.name,
                status: conn.status,
                addr: conn.addr,
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
