//! A record a normalizer refused. Upstream reads are destructive, so this row is the only
//! copy left, kept for a fortnight so someone can see what the gateway sent.

use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};

use crate::app::AppState;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::source::Source;

const RETENTION: SignedDuration = SignedDuration::from_hours(14 * 24);
const PRUNE_EVERY: Duration = Duration::from_hours(24);

#[derive(Debug, FromRow)]
pub struct DeadLetter {
    #[sqlx(rename = "dead_letter_id")]
    pub id: Id<DeadLetter>,
    pub source_id: Id<Source>,
    #[sqlx(try_from = "StoredTimestamp")]
    pub received_at: Timestamp,
    pub error: String,
    /// JSON text, already stripped of secrets.
    pub payload: String,
}

impl DeadLetter {
    pub async fn insert(
        connection: &mut SqliteConnection,
        id: Id<DeadLetter>,
        source_id: Id<Source>,
        received_at: Timestamp,
        error: &str,
        payload: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO dead_letter (dead_letter_id, source_id, received_at, error, payload) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(source_id)
        .bind(StoredTimestamp::from(received_at))
        .bind(error)
        .bind(payload.to_string())
        .execute(connection)
        .await?;

        Ok(())
    }

    /// Newest first.
    pub async fn list(
        database: &Database,
        source_id: Option<Id<Source>>,
        limit: i64,
    ) -> Result<Vec<Self>, DatabaseError> {
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT dead_letter_id, source_id, received_at, error, payload FROM dead_letter",
        );
        if let Some(source_id) = source_id {
            builder.push(" WHERE source_id = ").push_bind(source_id);
        }
        builder
            .push(" ORDER BY received_at DESC, dead_letter_id DESC LIMIT ")
            .push_bind(limit);

        Ok(builder
            .build_query_as::<Self>()
            .fetch_all(&database.pool)
            .await?)
    }

    /// Answers how many rows were older than the retention and went.
    pub async fn prune(database: &Database, now: Timestamp) -> Result<u64, DatabaseError> {
        let pruned = sqlx::query("DELETE FROM dead_letter WHERE received_at < ?")
            .bind(StoredTimestamp::from(now - RETENTION))
            .execute(&database.pool)
            .await?;

        Ok(pruned.rows_affected())
    }
}

/// Prunes once now and then once a day, for as long as the process runs.
pub fn spawn_pruning(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PRUNE_EVERY);
        loop {
            interval.tick().await;
            match DeadLetter::prune(&state.database, Timestamp::now()).await {
                Ok(0) => {}
                Ok(pruned) => tracing::info!(pruned, "expired dead letters were deleted"),
                Err(error) => tracing::error!(%error, "dead letters were not pruned"),
            }
        }
    });
}
