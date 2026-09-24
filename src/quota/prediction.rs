//! What a window holds now, from the provider's last report and the usage this service
//! metered since. A window that reset after the report is carried into the one that followed
//! it, so a report hours old still says something about now.

use jiff::{RoundMode, SignedDuration, Timestamp, TimestampRound, Unit};
use sqlx::FromRow;

use crate::account::Account;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::quota::window::{QuotaWindow, REQUESTS, Scope};

#[derive(Debug, Clone, Copy)]
pub struct Prediction {
    /// The share of the window used by now.
    pub used_fraction: f64,
    /// When the window that holds now resets. None for a window that opens on its next
    /// request and has had none since it reset.
    pub resets_at: Option<Timestamp>,
    /// The window reset after the report, so the report no longer describes it.
    pub has_reset: bool,
}

/// The share of the window one metered unit spends.
#[derive(Debug, Clone, Copy)]
enum Rate {
    PerRequest(f64),
    /// None until the account has a window to learn it from.
    PerUsd(Option<f64>),
}

#[derive(Debug, FromRow)]
struct Metered {
    requests: i32,
    cost_usd: f64,
}

impl Prediction {
    /// None when nothing was metered since the report of a window that has not reset, or
    /// when nothing relates metered usage to the window.
    pub async fn of(
        database: &Database,
        window: &QuotaWindow,
        now: Timestamp,
    ) -> Result<Option<Self>, DatabaseError> {
        if window.scope == Scope::Unmetered {
            return Ok(None);
        }
        let rate = if window.unit.as_deref() == Some(REQUESTS) {
            match window.limit_value.filter(|limit| *limit > 0.0) {
                Some(limit) => Rate::PerRequest(1.0 / limit),
                None => return Ok(None),
            }
        } else {
            Rate::PerUsd(
                window
                    .calibration
                    .as_ref()
                    .map(|calibration| calibration.fraction_per_usd),
            )
        };
        let resets_at = window.resets_at.map(|at| at.0);
        let Some(reset) = resets_at.filter(|at| *at <= now) else {
            let Some(used_fraction) = window.used_fraction else {
                return Ok(None);
            };
            let metered = Metered::between(database, window, window.observed_at, now).await?;
            let added = rate.spent(&metered).filter(|added| *added > 0.0);

            return Ok(added.map(|added| Self {
                used_fraction: (used_fraction + added).min(1.0),
                resets_at,
                has_reset: false,
            }));
        };
        let Some(length) = window.window_seconds.map(SignedDuration::from_secs) else {
            return Ok(None);
        };
        let current = if window.starts_on_use {
            opened_on_use(database, window.account_id, reset, length, now).await?
        } else {
            next_on_schedule(reset, length, now)
        };
        let Some((start, resets_at)) = current else {
            return Ok(Some(Self {
                used_fraction: 0.0,
                resets_at: None,
                has_reset: true,
            }));
        };
        let metered = Metered::between(database, window, start, now).await?;

        Ok(rate.spent(&metered).map(|used_fraction| Self {
            used_fraction: used_fraction.min(1.0),
            resets_at: Some(resets_at),
            has_reset: true,
        }))
    }
}

impl Rate {
    /// Unknown only when there is spend and nothing to price it by.
    fn spent(self, metered: &Metered) -> Option<f64> {
        match self {
            Self::PerRequest(rate) => Some(rate * f64::from(metered.requests)),
            Self::PerUsd(Some(rate)) => Some(rate * metered.cost_usd),
            Self::PerUsd(None) => (metered.cost_usd <= 0.0).then_some(0.0),
        }
    }
}

impl Metered {
    async fn between(
        database: &Database,
        window: &QuotaWindow,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Self, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(
            "SELECT COALESCE(SUM(failed = 0), 0) AS requests, \
             COALESCE(SUM(list_cost_usd), 0.0) AS cost_usd \
             FROM usage_event WHERE account_id = ?1 AND occurred_at >= ?2 AND occurred_at <= ?3 \
             AND (?4 IS NULL OR instr(lower(model), ?4) > 0)",
        )
        .bind(window.account_id)
        .bind(StoredTimestamp::from(from))
        .bind(StoredTimestamp::from(to))
        .bind(window.scope.model())
        .fetch_one(&database.pool)
        .await?)
    }
}

/// The start and reset of the window holding `now`, for a window that opens at the first
/// request after it resets. Claude documents its five-hour session as starting with the
/// first message, and its resets are seen to fall on the hour, so the window runs from that
/// request rounded down to the hour. None when no request came since the reset.
async fn opened_on_use(
    database: &Database,
    account_id: Id<Account>,
    mut reset: Timestamp,
    length: SignedDuration,
    now: Timestamp,
) -> Result<Option<(Timestamp, Timestamp)>, DatabaseError> {
    let hour = TimestampRound::new()
        .smallest(Unit::Hour)
        .mode(RoundMode::Trunc);
    loop {
        let first = sqlx::query_scalar::<_, Option<StoredTimestamp>>(
            "SELECT MIN(occurred_at) FROM usage_event \
             WHERE account_id = ? AND failed = 0 AND occurred_at >= ? AND occurred_at <= ?",
        )
        .bind(account_id)
        .bind(StoredTimestamp::from(reset))
        .bind(StoredTimestamp::from(now))
        .fetch_one(&database.pool)
        .await?;
        let Some(first) = first.map(|first| first.0) else {
            return Ok(None);
        };
        let Some(ends) = first
            .round(hour)
            .and_then(|start| start.checked_add(length))
            .ok()
        else {
            return Ok(None);
        };
        if ends > now {
            return Ok(Some((first, ends)));
        }
        reset = ends;
    }
}

/// The start and reset of the window holding `now`, for a window that follows the last one
/// straight away.
fn next_on_schedule(
    reset: Timestamp,
    length: SignedDuration,
    now: Timestamp,
) -> Option<(Timestamp, Timestamp)> {
    let lengths = now.duration_since(reset).as_secs() / length.as_secs().max(1) + 1;
    let resets_at = reset
        .checked_add(length.checked_mul(i32::try_from(lengths).ok()?)?)
        .ok()?;

    Some((resets_at.checked_sub(length).ok()?, resets_at))
}
