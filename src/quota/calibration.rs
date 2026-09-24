//! The share of a window that one list dollar spends, learned from the windows a credential
//! went through. A provider sometimes resets a window early, after an incident, as a
//! courtesy; such a window reports far too little used for what was spent. The median
//! resists it, and it is named as an outlier so that it can be seen.

use jiff::{SignedDuration, Timestamp, Unit};
use sqlx::{FromRow, SqliteConnection};

use crate::account::Account;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::quota::window::{NewWindow, QuotaWindow, REQUESTS, Scope};

/// Only windows reported this recently teach the rate, so a changed plan is learned in
/// four weeks.
const HORIZON: SignedDuration = SignedDuration::from_hours(28 * 24);
/// Below this share, the rounding of a whole percent is most of what was reported.
const MIN_USED_FRACTION: f64 = 0.05;
/// A window whose share per dollar is off the median by this factor, either way.
const OUTLIER_FACTOR: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct Calibration {
    /// The median over the windows reported in the last four weeks.
    pub fraction_per_usd: f64,
    /// The windows the median was taken over.
    pub windows: usize,
    /// Newest first.
    pub outliers: Vec<Outlier>,
}

#[derive(Debug, Clone)]
pub struct Outlier {
    pub started_at: Timestamp,
    pub fraction_per_usd: f64,
}

#[derive(FromRow)]
struct Sample {
    #[sqlx(try_from = "StoredTimestamp")]
    started_at: Timestamp,
    used_fraction: f64,
    cost_usd: f64,
}

impl Calibration {
    pub async fn learn(
        database: &Database,
        window: &QuotaWindow,
        now: Timestamp,
    ) -> Result<Option<Self>, DatabaseError> {
        if window.scope == Scope::Unmetered || window.unit.as_deref() == Some(REQUESTS) {
            return Ok(None);
        }
        let samples = sqlx::query_as::<_, Sample>(
            "SELECT quota_sample.started_at, quota_sample.used_fraction, \
             (SELECT COALESCE(SUM(usage_event.list_cost_usd), 0.0) FROM usage_event \
                 WHERE usage_event.account_id = quota_sample.account_id \
                 AND usage_event.occurred_at >= quota_sample.started_at \
                 AND usage_event.occurred_at <= quota_sample.observed_at \
                 AND (?3 IS NULL OR instr(lower(usage_event.model), ?3) > 0)) AS cost_usd \
             FROM quota_sample WHERE account_id = ?1 AND window_key = ?2 \
             AND observed_at >= ?4 AND used_fraction >= ?5 \
             ORDER BY started_at DESC",
        )
        .bind(window.account_id)
        .bind(&window.window_key)
        .bind(window.scope.model())
        .bind(StoredTimestamp::from(now - HORIZON))
        .bind(MIN_USED_FRACTION)
        .fetch_all(&database.pool)
        .await?;
        let rates = samples
            .into_iter()
            .filter(|sample| sample.cost_usd > 0.0)
            .map(|sample| (sample.started_at, sample.used_fraction / sample.cost_usd))
            .collect::<Vec<_>>();
        let mut sorted = rates.iter().map(|(_, rate)| *rate).collect::<Vec<_>>();
        sorted.sort_by(f64::total_cmp);
        let Some(fraction_per_usd) = median(&sorted) else {
            return Ok(None);
        };

        Ok(Some(Self {
            fraction_per_usd,
            windows: rates.len(),
            outliers: rates
                .into_iter()
                .filter(|(_, rate)| {
                    let ratio = rate / fraction_per_usd;
                    !(1.0 / OUTLIER_FACTOR..=OUTLIER_FACTOR).contains(&ratio)
                })
                .map(|(started_at, fraction_per_usd)| Outlier {
                    started_at,
                    fraction_per_usd,
                })
                .collect(),
        }))
    }

    /// Keeps the report as the latest of its window. A window counted in requests needs no
    /// learning, and one without a share used or a start teaches nothing.
    pub async fn record(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
        window: &NewWindow,
        observed_at: Timestamp,
    ) -> Result<(), DatabaseError> {
        if window.unit == Some(REQUESTS) {
            return Ok(());
        }
        let (Some(used_fraction), Some(resets_at), Some(seconds)) = (
            window.used_fraction,
            window.resets_at,
            window.window_seconds,
        ) else {
            return Ok(());
        };
        // Providers restate the reset with some jitter from one answer to the next, which
        // must not split one window into several.
        let Some(started_at) = resets_at
            .checked_sub(SignedDuration::from_secs(seconds))
            .and_then(|start| start.round(Unit::Minute))
            .ok()
        else {
            return Ok(());
        };
        sqlx::query(
            "INSERT INTO quota_sample (account_id, window_key, started_at, observed_at, \
             used_fraction) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (account_id, window_key, started_at) DO UPDATE SET \
             observed_at = excluded.observed_at, used_fraction = excluded.used_fraction",
        )
        .bind(account_id)
        .bind(&window.key)
        .bind(StoredTimestamp::from(started_at))
        .bind(StoredTimestamp::from(observed_at))
        .bind(used_fraction.clamp(0.0, 1.0))
        .execute(connection)
        .await?;

        Ok(())
    }
}

fn median(sorted: &[f64]) -> Option<f64> {
    let middle = sorted.len() / 2;
    let upper = *sorted.get(middle)?;

    Some(if sorted.len().is_multiple_of(2) {
        f64::midpoint(sorted[middle - 1], upper)
    } else {
        upper
    })
}
