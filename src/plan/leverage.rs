//! What a plan's list cost was worth against its price, one billing period at a time.

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use sqlx::FromRow;

use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::plan::Plan;

#[derive(Debug)]
pub struct PlanLeverage {
    pub plan: Plan,
    /// Newest first.
    pub periods: Vec<BillingPeriod>,
}

impl PlanLeverage {
    /// Every plan with at most `periods` of its latest billing periods up to `now`.
    pub async fn list(
        database: &Database,
        periods: usize,
        now: Timestamp,
    ) -> Result<Vec<Self>, DatabaseError> {
        let plans = Plan::list(database).await?;
        let mut leverage = Vec::with_capacity(plans.len());
        for plan in plans {
            let mut billed = Vec::new();
            for (start, end) in plan.periods(now).into_iter().rev().take(periods) {
                let usage = sqlx::query_as::<_, PeriodUsage>(
                    "SELECT COUNT(*) AS requests, \
                     COALESCE(SUM(total_tokens), 0) AS total_tokens, \
                     COALESCE(SUM(list_cost_usd), 0.0) AS list_cost_usd, \
                     CASE WHEN SUM(total_tokens) > 0 \
                         THEN ?1 * 1e6 / SUM(total_tokens) END AS effective_usd_per_mtok \
                     FROM usage_event \
                     WHERE account_id = ?2 AND occurred_at >= ?3 AND occurred_at < ?4",
                )
                .bind(plan.monthly_usd)
                .bind(plan.account.id)
                .bind(StoredTimestamp::from(start))
                .bind(StoredTimestamp::from(end))
                .fetch_one(&database.pool)
                .await?;
                billed.push(BillingPeriod {
                    start,
                    end,
                    complete: end <= now,
                    requests: usage.requests,
                    total_tokens: usage.total_tokens,
                    list_cost_usd: usage.list_cost_usd,
                    leverage: (plan.monthly_usd > 0.0)
                        .then(|| usage.list_cost_usd / plan.monthly_usd),
                    effective_usd_per_mtok: usage.effective_usd_per_mtok,
                });
            }
            leverage.push(Self {
                plan,
                periods: billed,
            });
        }

        Ok(leverage)
    }
}

#[derive(Debug)]
pub struct BillingPeriod {
    pub start: Timestamp,
    /// Exclusive: the next period's start, or the plan's end when that comes first.
    pub end: Timestamp,
    pub complete: bool,
    pub requests: i64,
    pub total_tokens: i64,
    pub list_cost_usd: f64,
    /// List cost over the plan's price; nothing when the plan is free.
    pub leverage: Option<f64>,
    /// The plan's price per million tokens used; nothing when no token was used.
    pub effective_usd_per_mtok: Option<f64>,
}

#[derive(FromRow)]
struct PeriodUsage {
    requests: i64,
    total_tokens: i64,
    list_cost_usd: f64,
    effective_usd_per_mtok: Option<f64>,
}

impl Plan {
    /// Every billing period that has started by `now`, oldest first. Each is a calendar
    /// month from the start's day of month, clamped to the last day of a shorter month.
    /// Every start is counted from the first so that a clamp in February does not move
    /// every later period to the 28th.
    #[must_use]
    pub fn periods(&self, now: Timestamp) -> Vec<(Timestamp, Timestamp)> {
        let end = self.period_end.map(Timestamp::from);
        let anchor = self.period_start.to_zoned(TimeZone::UTC);
        let month = |index: i64| {
            anchor
                .checked_add(index.months())
                .ok()
                .map(|start| start.timestamp())
        };
        let mut periods = Vec::new();
        let mut index = 0;
        while let Some(start) = month(index) {
            if start > now || end.is_some_and(|end| start >= end) {
                break;
            }
            let Some(next) = month(index + 1) else {
                break;
            };
            periods.push((start, end.map_or(next, |end| end.min(next))));
            index += 1;
        }
        periods
    }
}
