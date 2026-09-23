//! One limit a provider reported for a credential, normalized so that providers counting in
//! percent used, percent remaining, or amounts all read as a used fraction.

use std::collections::HashSet;

use jiff::Timestamp;
use sqlx::{FromRow, SqliteConnection};

use crate::account::Account;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::source::Source;

#[derive(Debug, FromRow)]
pub struct QuotaWindow {
    pub account_id: Id<Account>,
    pub window_key: String,
    pub label: String,
    pub used_fraction: Option<f64>,
    pub used_value: Option<f64>,
    pub limit_value: Option<f64>,
    pub unit: Option<String>,
    pub window_seconds: Option<i64>,
    pub resets_at: Option<StoredTimestamp>,
    #[sqlx(try_from = "StoredTimestamp")]
    pub observed_at: Timestamp,
}

impl QuotaWindow {
    /// In the order the provider reported them.
    pub async fn list(
        database: &Database,
        source_id: Option<Id<Source>>,
    ) -> Result<Vec<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(
            "SELECT quota_window.account_id, quota_window.window_key, quota_window.label, \
             quota_window.used_fraction, quota_window.used_value, quota_window.limit_value, \
             quota_window.unit, quota_window.window_seconds, quota_window.resets_at, \
             quota_window.observed_at \
             FROM quota_window JOIN account ON account.account_id = quota_window.account_id \
             WHERE ?1 IS NULL OR account.source_id = ?1 \
             ORDER BY quota_window.account_id, quota_window.rowid",
        )
        .bind(source_id)
        .fetch_all(&database.pool)
        .await?)
    }

    /// Replaces every window of the account. A key reported twice keeps its first report.
    pub async fn replace(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
        windows: &[NewWindow],
        observed_at: Timestamp,
    ) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM quota_window WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *connection)
            .await?;
        let observed_at = StoredTimestamp::from(observed_at);
        let mut keys = HashSet::new();
        for window in windows.iter().filter(|window| keys.insert(&window.key)) {
            sqlx::query(
                "INSERT INTO quota_window (account_id, window_key, label, used_fraction, \
                 used_value, limit_value, unit, window_seconds, resets_at, observed_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(account_id)
            .bind(&window.key)
            .bind(&window.label)
            .bind(
                window
                    .used_fraction
                    .map(|fraction| fraction.clamp(0.0, 1.0)),
            )
            .bind(window.used_value)
            .bind(window.limit_value)
            .bind(window.unit)
            .bind(window.window_seconds)
            .bind(window.resets_at.map(StoredTimestamp::from))
            .bind(observed_at)
            .execute(&mut *connection)
            .await?;
        }

        Ok(())
    }
}

/// A window as an adapter read it from a provider's answer.
#[derive(Debug, Default)]
pub struct NewWindow {
    pub key: String,
    pub label: String,
    /// 0 is untouched, 1 is exhausted.
    pub used_fraction: Option<f64>,
    pub used_value: Option<f64>,
    pub limit_value: Option<f64>,
    /// What `used_value` and `limit_value` count.
    pub unit: Option<&'static str>,
    pub window_seconds: Option<i64>,
    pub resets_at: Option<Timestamp>,
}
