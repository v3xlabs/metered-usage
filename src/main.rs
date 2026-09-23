use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::source::Source;
use metered_usage::usage::dead_letter;
use poem::Server;
use poem::listener::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_PORT: u16 = 3000;
const DEFAULT_DATABASE_URL: &str = "sqlite:metered-usage.db";
const DUMP_OPENAPI: &str = "--dump-openapi";
const TOKEN_VARIABLE: &str = "METERED_USAGE_TOKEN";
const CONFIG_VARIABLE: &str = "METERED_USAGE_CONFIG";

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == DUMP_OPENAPI)
    {
        let path = arguments.get(index + 1).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{DUMP_OPENAPI} needs a path to write to"),
            )
        })?;
        // The document describes the code, not the data, so this needs no stored database,
        // no port and no configured token. The state is never served, and a random token
        // keeps it from ever admitting anyone.
        let database = Database::open("sqlite::memory:", 0)
            .await
            .map_err(std::io::Error::other)?;
        let token =
            Token::try_from(rand::random::<u128>().to_string()).map_err(std::io::Error::other)?;
        let state = Arc::new(
            AppState::new(database, token, Config::default()).map_err(std::io::Error::other)?,
        );
        if let Some(parent) = Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }

        return std::fs::write(path, metered_usage::http::specification(&state));
    }

    let token = std::env::var(TOKEN_VARIABLE)
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{TOKEN_VARIABLE} must hold the shared access token: {error}"),
            )
        })
        .and_then(|token| {
            Token::try_from(token).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{TOKEN_VARIABLE}: {error}"),
                )
            })
        })?;
    let database_url = environment("DATABASE_URL", DEFAULT_DATABASE_URL);
    let address = match std::env::var("BIND_ADDRESS") {
        Ok(address) => address.parse::<SocketAddr>().map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid BIND_ADDRESS: {error}"),
            )
        })?,
        Err(std::env::VarError::NotPresent) => {
            SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))
        }
        Err(error) => return Err(std::io::Error::other(error)),
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = match std::env::var(CONFIG_VARIABLE) {
        Ok(path) => Config::load(Path::new(&path)).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{CONFIG_VARIABLE}: {error}"),
            )
        })?,
        Err(std::env::VarError::NotPresent) => {
            tracing::warn!("{CONFIG_VARIABLE} is not set, so no source is collected from");
            Config::default()
        }
        Err(error) => return Err(std::io::Error::other(error)),
    };
    let database = Database::open(&database_url, 0)
        .await
        .map_err(std::io::Error::other)?;
    Source::sync(&database, &config)
        .await
        .map_err(std::io::Error::other)?;
    let state = Arc::new(AppState::new(database, token, config).map_err(|error| {
        std::io::Error::other(format!(
            "no HTTP client, most likely no CA certificate store on this host: {error}"
        ))
    })?);
    dead_letter::spawn_pruning(Arc::clone(&state));
    metered_usage::collector::spawn_all(Arc::clone(&state));
    metered_usage::price::spawn(Arc::clone(&state));
    metered_usage::quota::spawn(Arc::clone(&state));
    tracing::info!(%address, "metered-usage is listening");

    Server::new(TcpListener::bind(address))
        .run(metered_usage::http::routes(&state))
        .await
}

fn environment(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}
