mod cli;
mod html;
mod http;
mod logger;
mod slow_compiler_workaround;

use std::io;
use warp::Filter;

use cli::cli;

const DOMAIN: &str = "proxy.hubfederation.com";
const OWNER: &str = "pedrobittencourt3@gmail.com";

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    let _ = logger::init_logger();

    cli::init_cli()?;

    // Describe server:
    let server = warp::get()
        .and(warp::path::end())
        .map(|| warp::reply::with_header(include_str!("index.html"), "Content-Type", "text/html"))
        .or(http::api())
        .with(warp::log("api"));

    // Run server:
    if cli().https {
        // Run certobot:
        let status = std::process::Command::new("certbot")
            .arg("certonly")
            .arg("--standalone")
            .arg("--non-interactive")
            .arg("--email")
            .arg(OWNER)
            .arg("--agree-tos")
            .arg("--domain")
            .arg(DOMAIN)
            .spawn().expect("failed to spawn certbot").wait().expect("failed to run certbot");
        assert_eq!(status.code().expect("there always is a code (?)"), 0);

        // Start server:
        let server = warp::serve(server)
            .tls()
            .key_path(format!("/etc/letsencrypt/live/{}/privkey.pem", DOMAIN))
            .cert_path(format!("/etc/letsencrypt/live/{}/fullchain.pem", DOMAIN))
            .run(([0, 0, 0, 0], cli().port.unwrap_or(443)));
        
        tokio::spawn(server).await?
    } else {
        // Start server:
        let server = warp::serve(server).run(([0, 0, 0, 0], cli().port.unwrap_or(8080)));
        tokio::spawn(server).await?;
    }

    Ok(())
}
