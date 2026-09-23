//! One window in figures: its totals, its daily rate, its busiest days.

use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::analytics::AnalyticsError;
use crate::analytics::bucket::Bucket;
use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::metrics::UsageMetrics;
use crate::analytics::query::{Aggregate, Column, aggregate};
use crate::prelude::*;

#[derive(Debug)]
pub struct Summary {
    pub metrics: UsageMetrics,
    /// Local days the window covers, from its first day to the day of its last instant.
    pub range_days: i32,
    /// Local days with at least one event.
    pub active_days: i64,
    pub daily_burn: DailyBurn,
    pub peak_day_by_cost: Option<PeakDay>,
    pub peak_day_by_tokens: Option<PeakDay>,
    /// Cache reads over all input; absent when there was no input.
    pub cache_hit_rate: Option<f64>,
    pub distinct: DistinctCounts,
}

/// The window's totals spread over every day it covers, active or not.
#[derive(Debug, Clone, Copy, Default)]
pub struct DailyBurn {
    pub list_cost_usd: f64,
    pub billed_cost_usd: f64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone)]
pub struct PeakDay {
    pub day: Date,
    pub list_cost_usd: f64,
    pub total_tokens: i64,
}

/// How many values of each dimension the window holds. Events without a session are not
/// a session; events without a harness count as the harness `unknown`.
#[derive(Debug, Clone, Copy, FromRow)]
pub struct DistinctCounts {
    pub models: i64,
    pub providers: i64,
    pub accounts: i64,
    pub harnesses: i64,
    pub sources: i64,
    pub sessions: i64,
}

#[derive(FromRow)]
struct Overall {
    #[sqlx(flatten)]
    distinct: DistinctCounts,
    active_days: i64,
    cache_hit_rate: Option<f64>,
}

#[derive(FromRow)]
struct DayRow {
    day: String,
    #[sqlx(flatten)]
    aggregate: Aggregate,
}

impl Summary {
    pub async fn load(
        database: &Database,
        filter: &AnalyticsFilter,
    ) -> Result<Self, AnalyticsError> {
        filter.check(database).await?;
        let days = aggregate(filter, &[(Column::Bucket(Bucket::Day), "day")], None)
            .build_query_as::<DayRow>()
            .fetch_all(&database.pool)
            .await?;

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(DISTINCT usage_event.model) AS models, \
             COUNT(DISTINCT usage_event.provider) AS providers, \
             COUNT(DISTINCT usage_event.account_id) AS accounts, \
             COUNT(DISTINCT COALESCE(usage_event.harness, 'unknown')) AS harnesses, \
             COUNT(DISTINCT usage_event.source_id) AS sources, \
             COUNT(DISTINCT usage_event.session_id) AS sessions, \
             COUNT(DISTINCT ",
        );
        Bucket::Day.push_expression(&mut builder, filter);
        builder.push(
            ") AS active_days, \
             TOTAL(usage_event.cache_read_tokens) / NULLIF(TOTAL(usage_event.input_tokens), 0) \
             AS cache_hit_rate FROM usage_event",
        );
        filter.push_where(&mut builder);
        let overall = builder
            .build_query_as::<Overall>()
            .fetch_one(&database.pool)
            .await?;

        let mut metrics = UsageMetrics::default();
        let mut peak_day_by_cost: Option<PeakDay> = None;
        let mut peak_day_by_tokens: Option<PeakDay> = None;
        let mut first_at: Option<Timestamp> = None;
        let mut parsed = days
            .iter()
            .map(|row| {
                row.day.parse::<Date>().map(|day| (day, row)).map_err(|_| {
                    DatabaseError::Unreadable {
                        field: "occurred_at",
                        value: row.day.clone(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        parsed.sort_by_key(|(day, _)| *day);
        for (day, row) in parsed {
            let day_metrics = &row.aggregate.metrics;
            metrics += day_metrics;
            first_at =
                Some(first_at.map_or(row.aggregate.first_at, |at| at.min(row.aggregate.first_at)));
            let peak = PeakDay {
                day,
                list_cost_usd: day_metrics.list_cost_usd,
                total_tokens: day_metrics.total_tokens,
            };
            if peak_day_by_cost
                .as_ref()
                .is_none_or(|best| peak.list_cost_usd > best.list_cost_usd)
            {
                peak_day_by_cost = Some(peak.clone());
            }
            if peak_day_by_tokens
                .as_ref()
                .is_none_or(|best| peak.total_tokens > best.total_tokens)
            {
                peak_day_by_tokens = Some(peak);
            }
        }

        let range_days = match filter.from.or(first_at) {
            Some(from) => range_days(filter, from)?,
            None => 0,
        };
        let daily_burn = if range_days > 0 {
            DailyBurn {
                list_cost_usd: metrics.list_cost_usd / f64::from(range_days),
                billed_cost_usd: metrics.billed_cost_usd / f64::from(range_days),
                total_tokens: metrics.total_tokens / i64::from(range_days),
            }
        } else {
            DailyBurn::default()
        };

        Ok(Self {
            metrics,
            range_days,
            active_days: overall.active_days,
            daily_burn,
            peak_day_by_cost,
            peak_day_by_tokens,
            cache_hit_rate: overall.cache_hit_rate,
            distinct: overall.distinct,
        })
    }
}

fn range_days(filter: &AnalyticsFilter, from: Timestamp) -> Result<i32, AnalyticsError> {
    let first = filter.offset.to_datetime(from).date();
    let last_instant = filter.to.checked_sub(SignedDuration::from_nanos(1))?;
    let last = filter.offset.to_datetime(last_instant).date();
    if last < first {
        return Ok(0);
    }

    Ok(first.until(last)?.get_days() + 1)
}
