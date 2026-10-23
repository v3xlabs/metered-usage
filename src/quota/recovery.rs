//! Non-spending expired-sign-in recovery, with durable per-account retry deadlines.
use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use sqlx::FromRow;

use crate::app::AppState;
use crate::database::codec::StoredTimestamp;
use crate::prelude::*;
use crate::quota::provider::Provider;
use crate::quota::sign_in::SignIn;

/// The first retry waits one gateway sync interval, avoiding a boot-time provider burst.
pub const INITIAL_DELAY: SignedDuration = SignedDuration::from_secs(180);
/// Bound retries for persistently expired credentials to four reads a day.
pub const MAX_DELAY: SignedDuration = SignedDuration::from_secs(6 * 60 * 60);
/// Saturate the count beyond the capped delay to prevent shift overflow.
const MAX_ATTEMPTS: i64 = 8;

#[derive(FromRow)]
pub struct Recovery {
    attempts: i64,
    next_attempt_at: StoredTimestamp,
}

impl Recovery {
    /// SQLite serializes reservations. Shared action claims coalesce concurrent human
    /// work before dispatch. Reserve before execution so interruption preserves backoff.
    /// Attempts count scheduled reservations, including ones served by shared human
    /// results. Reuse advances the deadline without another provider read.
    pub async fn claim_due(
        database: &Database,
        account_id: Id<Account>,
        now: Timestamp,
    ) -> Result<bool, DatabaseError> {
        let mut write = database.write().await?;
        let credential = sqlx::query_scalar::<_, String>(
            "SELECT account.provider FROM credential_state \
             JOIN account USING (account_id) JOIN source USING (source_id) \
             WHERE account_id = ? AND sign_in_error IS NOT NULL AND removed_at IS NULL \
             AND disabled = 0 AND hard_refreshable = 1 AND source.enabled = 1",
        )
        .bind(account_id)
        .fetch_optional(&mut *write)
        .await?;
        let Some(provider) = credential else {
            return Ok(false);
        };
        if !provider
            .parse::<Provider>()
            .is_ok_and(Provider::allows_automatic_recovery)
        {
            return Ok(false);
        }
        sqlx::query("INSERT INTO quota_recovery (account_id, next_attempt_at) VALUES (?, ?) ON CONFLICT DO NOTHING")
            .bind(account_id).bind(StoredTimestamp::from(now + INITIAL_DELAY)).execute(&mut *write).await?;
        let recovery = sqlx::query_as::<_, Self>(
            "SELECT attempts, next_attempt_at FROM quota_recovery WHERE account_id = ?",
        )
        .bind(account_id)
        .fetch_one(&mut *write)
        .await?;
        let due = recovery.next_attempt_at.0 <= now;
        if due {
            let attempts = (recovery.attempts + 1).min(MAX_ATTEMPTS);
            let seconds = (INITIAL_DELAY.as_secs() * (1_i64 << attempts)).min(MAX_DELAY.as_secs());
            sqlx::query(
                "UPDATE quota_recovery SET attempts = ?, next_attempt_at = ? WHERE account_id = ?",
            )
            .bind(attempts)
            .bind(StoredTimestamp::from(
                now + SignedDuration::from_secs(seconds),
            ))
            .bind(account_id)
            .execute(&mut *write)
            .await?;
        }
        write.commit().await?;
        Ok(due)
    }
}

/// Each candidate checks its durable deadline and safety policy, then claims the
/// shared action. Concurrent human claims share one worker; only a successful
/// provider observation clears backoff. Gateway actions and human reuse leave it intact.
pub async fn recover_expired(state: Arc<AppState>) {
    match SignIn::expired(&state.database).await {
        Ok(accounts) => {
            for account_id in accounts {
                crate::http::quota_action::automatic_recovery(Arc::clone(&state), account_id).await;
            }
        }
        Err(error) => tracing::error!(%error, "expired sign-ins could not be listed"),
    }
}
