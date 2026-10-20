//! Usage per UTC hour or day of a closed window, optionally split by one dimension, with
//! every group kept: nothing is ranked away or merged.

use std::collections::{BTreeMap, HashMap};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use sqlx::FromRow;

use crate::analytics::AnalyticsError;
use crate::analytics::bucket::Bucket;
use crate::analytics::dimension::Dimension;
use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::metrics::UsageMetrics;
use crate::analytics::query::{Aggregate, Column, aggregate};
use crate::prelude::*;

#[derive(Debug)]
pub struct Rollup {
    pub total: UsageMetrics,
    /// Every group with an event in the window, by key; `None` when ungrouped.
    pub groups: Option<Vec<RollupGroup>>,
    /// Every bucket that overlaps the window, oldest first, zero where it had no events.
    pub buckets: Vec<RollupBucket>,
}

#[derive(Debug)]
pub struct RollupBucket {
    pub start: Timestamp,
    pub total: UsageMetrics,
    /// The groups with an event in this bucket, by key; `None` when ungrouped.
    pub groups: Option<Vec<RollupGroup>>,
}

/// One group, keyed as [`Dimension::key`] spells it.
#[derive(Debug)]
pub struct RollupGroup {
    pub key: String,
    pub metrics: UsageMetrics,
}

#[derive(FromRow)]
struct CellRow {
    bucket_key: String,
    group_key: String,
    #[sqlx(flatten)]
    aggregate: Aggregate,
}

impl Rollup {
    /// The events in `[from, to)`, cut into UTC buckets of `step`. A window of more than
    /// [`crate::analytics::MAX_BUCKETS`] buckets is refused before anything is read, and
    /// `from` must come before `to`.
    pub async fn load(
        database: &Database,
        from: Timestamp,
        to: Timestamp,
        step: Bucket,
        group_by: Option<Dimension>,
    ) -> Result<Self, AnalyticsError> {
        let filter = AnalyticsFilter::window(Some(from), Some(to), TimeZone::UTC);
        let starts = step.starts(from, to, &filter.time_zone)?;
        // UTC never changes its offset, so no recorded span is needed to place a bucket.
        let shift = filter.shift(None);
        let group = group_by.map_or(Column::Expression("''"), Column::Key);
        let rows = aggregate(
            &filter,
            &[
                (Column::Bucket(step, &shift), "bucket_key"),
                (group, "group_key"),
            ],
            None,
        )
        .build_query_as::<CellRow>()
        .fetch_all(&database.pool)
        .await?;

        let bucket_index = starts
            .iter()
            .enumerate()
            .map(|(index, start)| (start.key.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut buckets = starts
            .iter()
            .map(|start| RollupBucket {
                start: start.start,
                total: UsageMetrics::default(),
                groups: group_by.map(|_| Vec::new()),
            })
            .collect::<Vec<_>>();
        let mut total = UsageMetrics::default();
        let mut groups = BTreeMap::<String, UsageMetrics>::new();
        for row in rows {
            let index = *bucket_index.get(row.bucket_key.as_str()).ok_or_else(|| {
                DatabaseError::Unreadable {
                    field: "occurred_at",
                    value: row.bucket_key.clone(),
                }
            })?;
            let metrics = row.aggregate.metrics;
            total += &metrics;
            let bucket = &mut buckets[index];
            bucket.total += &metrics;
            if let (Some(dimension), Some(cells)) = (group_by, bucket.groups.as_mut()) {
                let key = dimension.key(row.group_key)?;
                *groups.entry(key.clone()).or_default() += &metrics;
                cells.push(RollupGroup { key, metrics });
            }
        }
        // Rows come in no promised order, and an account's stored integer sorts as text
        // differently from its encoded id, so each bucket is ordered once its keys are final.
        for cells in buckets
            .iter_mut()
            .filter_map(|bucket| bucket.groups.as_mut())
        {
            cells.sort_unstable_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
        }

        Ok(Self {
            total,
            groups: group_by.map(|_| {
                groups
                    .into_iter()
                    .map(|(key, metrics)| RollupGroup { key, metrics })
                    .collect()
            }),
            buckets,
        })
    }
}
