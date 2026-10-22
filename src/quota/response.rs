//! Meta's subscription exhaustion as reported in a normal request's 429 body.
//!
//! `meta-subscription` is reserved for this evidence, not a measured usage window. It has
//! no duration or amount: a reset does not imply a new weekly window or a full balance.
//! Only a later successful request or the named reset clears it. Hard refreshes preserve
//! it, since a key response with no usage fields says nothing about request exhaustion.

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::Value;
use sqlx::SqliteConnection;
use tokio::time::MissedTickBehavior;

use crate::account::Account;
use crate::app::AppState;
use crate::change::snapshot::{Scope, Snapshot};
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::usage::NewUsageEvent;

pub const WINDOW_KEY: &str = "meta-subscription";
/// Clears expired evidence and notifies feed readers within one second of its reset.
const EXPIRE_EVERY: Duration = Duration::from_secs(1);

#[derive(Deserialize)]
struct ErrorBody {
    error: ProviderError,
}

#[derive(Deserialize)]
struct ProviderError {
    #[serde(default)]
    message: Value,
    #[serde(default)]
    code: Value,
    #[serde(default)]
    resets_at: Value,
}

impl ProviderError {
    fn reset(&self) -> Option<Timestamp> {
        let message = self.message.as_str().unwrap_or_default();
        let code = self.code.as_str().unwrap_or_default();
        let quota_message = contains_ascii_case_insensitive(message, b"subscription quota")
            || contains_ascii_case_insensitive(message, b"quota exhausted");
        let quota_code = code.eq_ignore_ascii_case("rate_limit_exceeded")
            || contains_ascii_case_insensitive(code, b"quota");
        if !quota_message && !quota_code {
            return None;
        }
        let seconds = self.resets_at.as_i64().or_else(|| {
            self.resets_at
                .as_str()
                .and_then(|seconds| seconds.parse().ok())
        });
        seconds
            .and_then(|seconds| Timestamp::from_second(seconds).ok())
            .or_else(|| {
                message
                    .split_once("Your usage window resets at ")?
                    .1
                    .trim_end_matches('.')
                    .parse()
                    .ok()
            })
    }
}

fn contains_ascii_case_insensitive(text: &str, needle: &[u8]) -> bool {
    text.as_bytes()
        .windows(needle.len())
        .any(|part| part.eq_ignore_ascii_case(needle))
}

/// Runs only for newly inserted records, in the same transaction as their quota changes.
pub async fn observe(
    connection: &mut SqliteConnection,
    account_id: Id<Account>,
    event: &NewUsageEvent,
    now: Timestamp,
) -> Result<(), DatabaseError> {
    if event.provider != "meta" {
        return Ok(());
    }
    let is_success = !event.failed && (200..300).contains(&event.status_code);
    let reset = if event.failed && event.status_code == 429 {
        event.error_message.as_deref().and_then(|body| {
            serde_json::from_str::<ErrorBody>(body)
                .ok()?
                .error
                .reset()
                .filter(|at| *at > now)
        })
    } else {
        None
    };
    if !is_success && reset.is_none() {
        return Ok(());
    }
    // Buffered failures must not resurrect exhaustion after a newer success, nor may an
    // older success clear a newer failure. The usage history also guards a deleted marker.
    let newer: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM usage_event WHERE account_id = ? AND occurred_at > ? \
         AND failed = 0 AND status_code >= 200 AND status_code < 300)",
    )
    .bind(account_id)
    .bind(StoredTimestamp::from(event.occurred_at))
    .fetch_one(&mut *connection)
    .await?;
    if newer {
        return Ok(());
    }
    if let Some(reset) = reset {
        sqlx::query(
            "INSERT INTO quota_window (account_id, window_key, label, used_fraction, \
             resets_at, observed_at, scope, starts_on_use) \
             VALUES (?, ?, 'Subscription quota', 1, ?, ?, 'unmetered', 0) \
             ON CONFLICT (account_id, window_key) DO UPDATE SET \
             resets_at = excluded.resets_at, observed_at = excluded.observed_at \
             WHERE excluded.observed_at >= quota_window.observed_at",
        )
        .bind(account_id)
        .bind(WINDOW_KEY)
        .bind(StoredTimestamp::from(reset))
        .bind(StoredTimestamp::from(event.occurred_at))
        .execute(connection)
        .await?;
    } else {
        sqlx::query(
            "DELETE FROM quota_window WHERE account_id = ? AND window_key = ? AND observed_at <= ?",
        )
        .bind(account_id)
        .bind(WINDOW_KEY)
        .bind(StoredTimestamp::from(event.occurred_at))
        .execute(connection)
        .await?;
    }
    Ok(())
}

/// Deletes expired evidence with the existing `quota.updated` invalidation.
pub async fn expire(database: &Database, now: Timestamp) -> Result<(), DatabaseError> {
    let has_expired: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM quota_window WHERE window_key = ? AND resets_at <= ?)",
    )
    .bind(WINDOW_KEY)
    .bind(StoredTimestamp::from(now))
    .fetch_one(&database.pool)
    .await?;
    if !has_expired {
        return Ok(());
    }
    let mut transaction = database.write().await?;
    let accounts = sqlx::query_scalar::<_, Id<Account>>(
        "SELECT account_id FROM quota_window WHERE window_key = ? AND resets_at <= ?",
    )
    .bind(WINDOW_KEY)
    .bind(StoredTimestamp::from(now))
    .fetch_all(&mut *transaction)
    .await?;
    for account_id in accounts {
        let snapshot = Snapshot::quota(&mut transaction, Scope::Account(account_id)).await?;
        sqlx::query("DELETE FROM quota_window WHERE account_id = ? AND window_key = ?")
            .bind(account_id)
            .bind(WINDOW_KEY)
            .execute(&mut *transaction)
            .await?;
        snapshot.record_changes(&mut transaction).await?;
    }
    transaction.commit().await?;
    Ok(())
}

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(EXPIRE_EVERY);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = expire(&state.database, Timestamp::now()).await {
                tracing::error!(%error, "response quota expiry failed");
            }
        }
    });
}
