use tokio::sync::broadcast;

use crate::config::Config;
use crate::database::Database;
use crate::http::auth::Token;
use crate::usage::UsageEvent;

/// How many committed events a slow stream subscriber may fall behind before it is cut
/// off and left to catch up from the database on reconnect.
const USAGE_BACKLOG: usize = 1024;

/// Everything a request handler needs.
pub struct AppState {
    pub database: Database,
    pub token: Token,
    pub config: Config,
    /// Every event ingest committed, in commit order per batch.
    pub usage: broadcast::Sender<UsageEvent>,
    /// Shared by every outbound call so connections are pooled.
    pub http: reqwest::Client,
}

impl AppState {
    /// Fails when the platform has no usable certificate store, which `reqwest::Client::new`
    /// would answer with a panic instead.
    pub fn new(database: Database, token: Token, config: Config) -> Result<Self, reqwest::Error> {
        Ok(Self {
            database,
            token,
            config,
            usage: broadcast::channel(USAGE_BACKLOG).0,
            http: reqwest::Client::builder().build()?,
        })
    }
}
