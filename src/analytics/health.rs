//! Requests and failures per account over the last stretch of time, in buckets aligned to
//! whole multiples of their length since the Unix epoch, so consecutive polls line up.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::tz::Offset;
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::analytics::AnalyticsError;
use crate::analytics::filter::AnalyticsFilter;
use crate::prelude::*;

pub const MIN_WINDOW_MINUTES: u32 = 5;
pub const MAX_WINDOW_MINUTES: u32 = 1440;
pub const MAX_HEALTH_BUCKETS: u32 = 288;
const SECONDS_PER_MINUTE: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HealthWindowError {
    #[error("window_minutes must be from {MIN_WINDOW_MINUTES} to {MAX_WINDOW_MINUTES}")]
    Window,
    #[error("bucket_minutes must be at least 1")]
    Bucket,
    #[error("window_minutes must be a multiple of bucket_minutes")]
    Indivisible,
    #[error(
        "the window holds more than {MAX_HEALTH_BUCKETS} buckets; choose a wider bucket or a shorter window"
    )]
    TooManyBuckets,
}

/// A window that divides into at most [`MAX_HEALTH_BUCKETS`] whole buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthWindow {
    bucket_minutes: u32,
    buckets: u32,
}

#[derive(Debug)]
pub struct Health {
    pub bucket_minutes: u32,
    pub buckets_start: Timestamp,
    /// By account id.
    pub accounts: Vec<AccountHealth>,
}

#[derive(Debug)]
pub struct AccountHealth {
    pub account_id: Id<Account>,
    /// Oldest first, every bucket of the window.
    pub buckets: Vec<HealthBucket>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthBucket {
    pub start: Timestamp,
    pub requests: i64,
    pub failures: i64,
}

#[derive(FromRow)]
struct HealthRow {
    account_id: Id<Account>,
    bucket: i64,
    requests: i64,
    failures: i64,
}

impl HealthWindow {
    pub fn new(window_minutes: u32, bucket_minutes: u32) -> Result<Self, HealthWindowError> {
        if !(MIN_WINDOW_MINUTES..=MAX_WINDOW_MINUTES).contains(&window_minutes) {
            return Err(HealthWindowError::Window);
        }
        if bucket_minutes == 0 {
            return Err(HealthWindowError::Bucket);
        }
        if !window_minutes.is_multiple_of(bucket_minutes) {
            return Err(HealthWindowError::Indivisible);
        }
        let buckets = window_minutes / bucket_minutes;
        if buckets > MAX_HEALTH_BUCKETS {
            return Err(HealthWindowError::TooManyBuckets);
        }

        Ok(Self {
            bucket_minutes,
            buckets,
        })
    }
}

impl Health {
    /// The window ends with the bucket that holds `now`. An account without a request in
    /// it is reported only when `account_ids` names it.
    pub async fn load(
        database: &Database,
        window: HealthWindow,
        now: Timestamp,
        source_ids: Vec<Id<Source>>,
        account_ids: Vec<Id<Account>>,
    ) -> Result<Self, AnalyticsError> {
        let bucket_seconds = i64::from(window.bucket_minutes) * SECONDS_PER_MINUTE;
        let end = (now.as_second().div_euclid(bucket_seconds) + 1) * bucket_seconds;
        let start = end - bucket_seconds * i64::from(window.buckets);
        let buckets_start = Timestamp::from_second(start)?;
        let filter = AnalyticsFilter {
            source_ids,
            account_ids,
            ..AnalyticsFilter::window(
                Some(buckets_start),
                Some(Timestamp::from_second(end)?),
                Offset::UTC,
            )
        };
        filter.check(database).await?;

        // The stored instant is cut to whole seconds first, because SQLite rounds a
        // fraction to the millisecond and would move the last instant of a bucket into the
        // next one.
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT usage_event.account_id, \
             (unixepoch(substr(usage_event.occurred_at, 1, 19)) - ",
        );
        builder
            .push_bind(start)
            .push(") / ")
            .push_bind(bucket_seconds)
            .push(
                " AS bucket, COUNT(*) AS requests, SUM(usage_event.failed) AS failures \
                 FROM usage_event",
            );
        filter.push_where(&mut builder);
        builder.push(" GROUP BY usage_event.account_id, bucket");
        let rows = builder
            .build_query_as::<HealthRow>()
            .fetch_all(&database.pool)
            .await?;

        let empty = (0..i64::from(window.buckets))
            .map(|index| {
                Ok(HealthBucket {
                    start: Timestamp::from_second(start + index * bucket_seconds)?,
                    requests: 0,
                    failures: 0,
                })
            })
            .collect::<Result<Vec<_>, jiff::Error>>()?;
        let mut accounts = filter
            .account_ids
            .iter()
            .map(|id| (*id, empty.clone()))
            .collect::<BTreeMap<_, _>>();
        for row in rows {
            let bucket = usize::try_from(row.bucket)
                .ok()
                .and_then(|index| {
                    accounts
                        .entry(row.account_id)
                        .or_insert_with(|| empty.clone())
                        .get_mut(index)
                })
                .ok_or_else(|| DatabaseError::Unreadable {
                    field: "occurred_at",
                    value: row.bucket.to_string(),
                })?;
            bucket.requests = row.requests;
            bucket.failures = row.failures;
        }

        Ok(Self {
            bucket_minutes: window.bucket_minutes,
            buckets_start,
            accounts: accounts
                .into_iter()
                .map(|(account_id, buckets)| AccountHealth {
                    account_id,
                    buckets,
                })
                .collect(),
        })
    }
}
