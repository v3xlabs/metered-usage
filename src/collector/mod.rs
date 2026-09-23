//! The resident tasks that read each configured gateway, one per enabled source, and the
//! row in `collector_state` each keeps current about itself.

pub mod cliproxy;
pub mod litellm;
pub mod resp;

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;

use crate::app::AppState;
use crate::collector::resp::RespError;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::source::{Source, SourceKind};
use crate::usage::ingest::IngestReport;

const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(60);

/// Why a collector gave up on its connection or its poll. None of these carries a secret:
/// replies are described by their kind, never quoted, because a reply can be a record.
#[derive(Debug, thiserror::Error)]
pub enum CollectorError {
    #[error("{0}")]
    Resp(#[from] RespError),
    #[error("{command} was refused: {message}")]
    Refused {
        command: &'static str,
        message: String,
    },
    #[error("{command} was answered with {reply}")]
    Unexpected {
        command: &'static str,
        reply: &'static str,
    },
    #[error("{0}")]
    Database(#[from] DatabaseError),
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("LiteLLM answered {0}")]
    Status(reqwest::StatusCode),
    #[error("base_url {0:?} names no host and port")]
    Address(String),
}

/// Waits 1 s after the first failure, doubling to a minute.
#[derive(Debug)]
pub struct Backoff {
    delay: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { delay: FIRST_RETRY }
    }
}

impl Backoff {
    pub fn reset(&mut self) {
        self.delay = FIRST_RETRY;
    }

    pub async fn wait(&mut self) {
        tokio::time::sleep(self.delay).await;
        self.delay = (self.delay * 2).min(LAST_RETRY);
    }
}

/// The collector's own row in `collector_state`.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub source_id: Id<Source>,
}

impl Progress {
    /// A new subscription: the connection is as old as this moment.
    pub async fn connected(&self, database: &Database) -> Result<(), DatabaseError> {
        self.upsert(
            database,
            "connected_since = excluded.updated_at, consecutive_failures = 0, last_error = NULL",
        )
        .await
    }

    /// A poll that worked. A poller has no connection, so it counts as connected since
    /// the first poll after its last failure.
    pub async fn polled(&self, database: &Database) -> Result<(), DatabaseError> {
        self.upsert(
            database,
            "connected_since = COALESCE(collector_state.connected_since, excluded.updated_at), \
             consecutive_failures = 0, last_error = NULL",
        )
        .await
    }

    /// Counts the records a batch brought in, stored or dead-lettered, not those it had
    /// already seen.
    pub async fn received(
        &self,
        database: &Database,
        report: &IngestReport,
    ) -> Result<(), DatabaseError> {
        let received = report.accepted + report.rejected;
        if received == 0 {
            return Ok(());
        }
        let now = StoredTimestamp::from(Timestamp::now());
        sqlx::query(
            "INSERT INTO collector_state (source_id, last_record_at, records_received, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (source_id) DO UPDATE SET last_record_at = excluded.last_record_at, \
             records_received = collector_state.records_received + excluded.records_received, \
             updated_at = excluded.updated_at",
        )
        .bind(self.source_id)
        .bind(now)
        .bind(received)
        .bind(now)
        .execute(&database.pool)
        .await?;
        Ok(())
    }

    pub async fn failed(&self, database: &Database, error: &str) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO collector_state (source_id, consecutive_failures, last_error, updated_at) \
             VALUES (?, 1, ?, ?) \
             ON CONFLICT (source_id) DO UPDATE SET connected_since = NULL, \
             consecutive_failures = collector_state.consecutive_failures + 1, \
             last_error = excluded.last_error, updated_at = excluded.updated_at",
        )
        .bind(self.source_id)
        .bind(error)
        .bind(StoredTimestamp::from(Timestamp::now()))
        .execute(&database.pool)
        .await?;
        Ok(())
    }

    /// Records a failure, logs it once with the source key, and waits out the backoff.
    pub async fn retry_after(
        &self,
        database: &Database,
        key: &str,
        error: &CollectorError,
        backoff: &mut Backoff,
    ) {
        let error = error.to_string();
        tracing::warn!(source = key, %error, "collector failed, retrying");
        if let Err(stored) = self.failed(database, &error).await {
            tracing::error!(source = key, error = %stored, "could not record the collector failure");
        }
        backoff.wait().await;
    }

    /// `set` names the columns that differ between the two ways of being healthy.
    async fn upsert(&self, database: &Database, set: &'static str) -> Result<(), DatabaseError> {
        let now = StoredTimestamp::from(Timestamp::now());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO collector_state (source_id, connected_since, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT (source_id) DO UPDATE SET {set}, updated_at = excluded.updated_at"
        )))
        .bind(self.source_id)
        .bind(now)
        .bind(now)
        .execute(&database.pool)
        .await?;
        Ok(())
    }
}

/// Starts one collector per configured source that is enabled, and returns at once.
pub fn spawn_all(state: Arc<AppState>) {
    let count = state.config.sources.len();
    for (index, state) in std::iter::repeat_n(state, count).enumerate() {
        tokio::spawn(start(state, index));
    }
}

async fn start(state: Arc<AppState>, index: usize) {
    let config = &state.config.sources[index];
    let source = match Source::by_key(&state.database, &config.key).await {
        Ok(Some(source)) if source.enabled => source,
        Ok(Some(_)) => return,
        Ok(None) => {
            tracing::error!(
                source = config.key,
                "configured source has no row, not collecting"
            );
            return;
        }
        Err(error) => {
            tracing::error!(source = config.key, %error, "could not load source, not collecting");
            return;
        }
    };
    tracing::info!(source = config.key, kind = %config.kind, "collector started");
    match config.kind {
        SourceKind::CliProxy => cliproxy::run(&state, source.id, config).await,
        SourceKind::LiteLlm => litellm::run(&state, source.id, config).await,
    }
}
