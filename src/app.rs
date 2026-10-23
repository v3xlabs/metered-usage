use tokio::sync::{Mutex, broadcast};

use crate::config::Config;
use crate::database::Database;
use crate::http::auth::Credentials;
use crate::usage::UsageUpdate;

/// How many committed events a slow stream subscriber may fall behind before it is cut
/// off and left to catch up from the database on reconnect.
const USAGE_BACKLOG: usize = 1024;

/// Everything a request handler needs.
pub struct AppState {
    pub database: Database,
    pub credentials: Credentials,
    pub config: Config,
    /// Every event ingest committed, in commit order per batch, and every cost a price
    /// fill committed afterwards.
    pub usage: broadcast::Sender<UsageUpdate>,
    /// Shared by every outbound call so connections are pooled.
    pub http: reqwest::Client,
    /// Soft sync, cooldown and provider-action families have separate claim locks.
    /// Provider refresh and reset share a family to serialize spending and its observation.
    pub quota_action_locks: [Mutex<()>; 3],
    pub quota_action_owners: crate::http::quota_action_owner::ActionOwners,
    pub manual_reset_lock: Mutex<()>,
}

impl AppState {
    /// Fails when the platform has no usable certificate store, which `reqwest::Client::new`
    /// would answer with a panic instead.
    pub fn new(
        database: Database,
        credentials: Credentials,
        config: Config,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            database,
            credentials,
            config,
            usage: broadcast::channel(USAGE_BACKLOG).0,
            http: reqwest::Client::builder().build()?,
            quota_action_locks: std::array::from_fn(|_| Mutex::new(())),
            quota_action_owners: crate::http::quota_action_owner::ActionOwners::default(),
            manual_reset_lock: Mutex::new(()),
        })
    }
}
