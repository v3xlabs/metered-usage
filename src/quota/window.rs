//! One limit a provider reported for a credential, normalized so that providers counting in
//! percent used, percent remaining, or amounts all read as a used fraction.

use std::collections::{HashMap, HashSet};
use std::fmt;

use jiff::{SignedDuration, Timestamp};
use sqlx::{FromRow, SqliteConnection};

use crate::account::Account;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::quota::calibration::Calibration;
use crate::quota::prediction::Prediction;
use crate::quota::response::WINDOW_KEY;
use crate::source::Source;

/// Kimi's unit, and the one unit this service meters too: each request it records is one
/// the provider counts.
pub const REQUESTS: &str = "requests";

/// A provider that gives a reset as seconds from now, as Codex and Kimi can, is stored as
/// the time of the answer plus those seconds, so two answers about one reset differ by a
/// second or two of request time. A stored reset this close to the reported one is kept,
/// so the same report reads the same and records no change. Real resets are minutes apart.
const RESET_TOLERANCE: SignedDuration = SignedDuration::from_secs(5);

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
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub starts_on_use: bool,
    #[sqlx(skip)]
    pub calibration: Option<Calibration>,
    #[sqlx(skip)]
    pub prediction: Option<Prediction>,
}

impl QuotaWindow {
    /// Response-derived exhaustion first, then measured windows in provider order.
    pub async fn list(
        database: &Database,
        source_id: Option<Id<Source>>,
    ) -> Result<Vec<Self>, DatabaseError> {
        let mut windows = sqlx::query_as::<_, Self>(
            "SELECT quota_window.account_id, quota_window.window_key, quota_window.label, \
             quota_window.used_fraction, quota_window.used_value, quota_window.limit_value, \
             quota_window.unit, quota_window.window_seconds, quota_window.resets_at, \
             quota_window.observed_at, quota_window.scope, quota_window.starts_on_use \
             FROM quota_window JOIN account ON account.account_id = quota_window.account_id \
             WHERE (?1 IS NULL OR account.source_id = ?1) \
             AND (window_key != ?2 OR resets_at > ?3) \
             ORDER BY quota_window.account_id, quota_window.window_key != ?2, quota_window.rowid",
        )
        .bind(source_id)
        .bind(WINDOW_KEY)
        .bind(StoredTimestamp::from(Timestamp::now()))
        .fetch_all(&database.pool)
        .await?;
        let now = Timestamp::now();
        for window in &mut windows {
            window.calibration = Calibration::learn(database, window, now).await?;
            window.prediction = Prediction::of(database, window, now).await?;
        }

        Ok(windows)
    }

    /// Replaces every window of the account. A key reported twice keeps its first report. A
    /// stored reset within [`RESET_TOLERANCE`] of the reported one is kept.
    pub async fn replace(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
        windows: &[NewWindow],
        observed_at: Timestamp,
    ) -> Result<(), DatabaseError> {
        let stored = sqlx::query_as::<_, (String, Option<StoredTimestamp>)>(
            "SELECT window_key, resets_at FROM quota_window WHERE account_id = ?",
        )
        .bind(account_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .collect::<HashMap<_, _>>();
        sqlx::query("DELETE FROM quota_window WHERE account_id = ? AND window_key != ?")
            .bind(account_id)
            .bind(WINDOW_KEY)
            .execute(&mut *connection)
            .await?;
        let observed_at = StoredTimestamp::from(observed_at);
        let mut keys = HashSet::new();
        for window in windows.iter().filter(|window| keys.insert(&window.key)) {
            sqlx::query(
                "INSERT INTO quota_window (account_id, window_key, label, used_fraction, \
                 used_value, limit_value, unit, window_seconds, resets_at, observed_at, scope, \
                 starts_on_use) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .bind(window.resets_at.map(
                |reported| match stored.get(&window.key).copied().flatten() {
                    Some(kept) if reported.duration_since(kept.0).abs() <= RESET_TOLERANCE => kept,
                    _ => StoredTimestamp::from(reported),
                },
            ))
            .bind(observed_at)
            .bind(window.scope.to_string())
            .bind(window.starts_on_use)
            .execute(&mut *connection)
            .await?;
            Calibration::record(&mut *connection, account_id, window, observed_at.0).await?;
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
    pub scope: Scope,
    /// The window opens at the first request after it resets, not at the reset.
    pub starts_on_use: bool,
}

/// The metered usage that counts toward a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Account,
    /// Only the models whose lowercased name contains this.
    Model(String),
    /// None of it: the window counts work that never passes a metered gateway.
    Unmetered,
}

impl Scope {
    /// What a model name must contain to count, where the scope is a model.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Model(model) => Some(model),
            Self::Account | Self::Unmetered => None,
        }
    }
}

impl From<String> for Scope {
    fn from(stored: String) -> Self {
        match stored.strip_prefix("model:") {
            Some(model) => Self::Model(model.to_owned()),
            None if stored == "unmetered" => Self::Unmetered,
            None => Self::Account,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account => formatter.write_str("account"),
            Self::Model(model) => write!(formatter, "model:{model}"),
            Self::Unmetered => formatter.write_str("unmetered"),
        }
    }
}
