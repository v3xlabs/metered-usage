//! One limit a provider reported for a credential, normalized so that providers counting in
//! percent used, percent remaining, or amounts all read as a used fraction.

use std::collections::HashSet;

use jiff::{SignedDuration, Timestamp};
use sqlx::{FromRow, SqliteConnection};

use crate::account::Account;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::source::Source;

/// Kimi's unit, and the one unit this service meters too: each request it records is one
/// the provider counts.
pub const REQUESTS: &str = "requests";

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
    /// The share used by now, estimated from what this service metered on the account since
    /// `observed_at`. None when nothing relates that usage to the window, or once it reset.
    #[sqlx(skip)]
    pub estimated_used_fraction: Option<f64>,
}

impl QuotaWindow {
    /// In the order the provider reported them.
    pub async fn list(
        database: &Database,
        source_id: Option<Id<Source>>,
    ) -> Result<Vec<Self>, DatabaseError> {
        let mut windows = sqlx::query_as::<_, Self>(
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
        .await?;
        let now = Timestamp::now();
        for window in &mut windows {
            window.estimated_used_fraction = window.estimate(database, now).await?;
        }

        Ok(windows)
    }

    /// A window counted in requests adds the requests metered since the observation. Any
    /// other assumes the share per list dollar the account spent in the window up to the
    /// observation holds after it; usage that bypasses the metered gateways inflates that
    /// share, so the estimate errs high.
    async fn estimate(
        &self,
        database: &Database,
        now: Timestamp,
    ) -> Result<Option<f64>, DatabaseError> {
        if self.resets_at.is_some_and(|at| at.0 <= now) {
            return Ok(None);
        }
        let Some(basis) = self.basis() else {
            return Ok(None);
        };
        let from = match basis {
            Basis::Requests { .. } => self.observed_at,
            Basis::Spend { start, .. } => start,
        };
        let metered = sqlx::query_as::<_, Metered>(
            "SELECT COALESCE(SUM(failed = 0 AND occurred_at > ?2), 0) AS requests_since, \
             COALESCE(SUM(CASE WHEN occurred_at > ?2 THEN list_cost_usd END), 0.0) \
                 AS cost_since_usd, \
             COALESCE(SUM(CASE WHEN occurred_at <= ?2 THEN list_cost_usd END), 0.0) \
                 AS cost_before_usd \
             FROM usage_event WHERE account_id = ?1 AND occurred_at >= ?3",
        )
        .bind(self.account_id)
        .bind(StoredTimestamp::from(self.observed_at))
        .bind(StoredTimestamp::from(from))
        .fetch_one(&database.pool)
        .await?;
        let estimate = match basis {
            Basis::Requests { used, limit } => {
                Some((used + f64::from(metered.requests_since)) / limit)
            }
            Basis::Spend { used_fraction, .. } => (metered.cost_before_usd > 0.0)
                .then(|| used_fraction * (1.0 + metered.cost_since_usd / metered.cost_before_usd)),
        };

        Ok(estimate.map(|fraction| fraction.min(1.0)))
    }

    fn basis(&self) -> Option<Basis> {
        if self.unit.as_deref() == Some(REQUESTS) {
            return Some(Basis::Requests {
                used: self.used_value?,
                limit: self.limit_value.filter(|limit| *limit > 0.0)?,
            });
        }
        // A share of zero says nothing of how fast the window fills.
        let used_fraction = self.used_fraction.filter(|fraction| *fraction > 0.0)?;
        let start = self
            .resets_at?
            .0
            .checked_sub(SignedDuration::from_secs(self.window_seconds?))
            .ok()?;

        (start < self.observed_at).then_some(Basis::Spend {
            used_fraction,
            start,
        })
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

/// How the usage this service metered maps onto a window.
#[derive(Debug, Clone, Copy)]
enum Basis {
    Requests {
        used: f64,
        limit: f64,
    },
    Spend {
        used_fraction: f64,
        start: Timestamp,
    },
}

#[derive(Debug, FromRow)]
struct Metered {
    requests_since: i32,
    cost_since_usd: f64,
    cost_before_usd: f64,
}
