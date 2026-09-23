//! Usage over time, one row per bucket and key, with no bucket missing.

use std::collections::HashMap;

use jiff::Timestamp;
use sqlx::FromRow;

use crate::analytics::AnalyticsError;
use crate::analytics::bucket::Bucket;
use crate::analytics::dimension::Dimension;
use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::metrics::{RankBy, UsageMetrics};
use crate::analytics::query::{Aggregate, Column, aggregate};
use crate::prelude::*;

/// The key of every row of an ungrouped series.
pub const ALL: &str = "all";
/// The key the groups outside the top are merged into.
pub const OTHER: &str = "other";

#[derive(Debug, Clone, Copy)]
pub struct SeriesQuery {
    pub bucket: Bucket,
    pub group_by: Option<Dimension>,
    /// How many groups keep their own key.
    pub top: usize,
    pub rank_by: RankBy,
}

#[derive(Debug)]
pub struct SeriesBucket {
    pub start: Timestamp,
    pub key: String,
    pub metrics: UsageMetrics,
}

#[derive(FromRow)]
struct SeriesRow {
    bucket_key: String,
    group_key: String,
    #[sqlx(flatten)]
    aggregate: Aggregate,
}

impl SeriesBucket {
    /// Bucket by bucket, and within a bucket in rank order with `other` last. Every
    /// returned key has every bucket of the window, zero where it had no events.
    pub async fn load(
        database: &Database,
        filter: &AnalyticsFilter,
        query: SeriesQuery,
    ) -> Result<Vec<Self>, AnalyticsError> {
        filter.check(database).await?;
        let group = query
            .group_by
            .map_or(Column::Expression("'all'"), Column::Key);
        let rows = aggregate(
            filter,
            &[
                (Column::Bucket(query.bucket), "bucket_key"),
                (group, "group_key"),
            ],
            None,
        )
        .build_query_as::<SeriesRow>()
        .fetch_all(&database.pool)
        .await?;

        let Some(from) = filter
            .from
            .or_else(|| rows.iter().map(|row| row.aggregate.first_at).min())
        else {
            return Ok(Vec::new());
        };
        let starts = query.bucket.starts(from, filter.to, filter.offset)?;

        let mut totals = HashMap::<&str, UsageMetrics>::new();
        for row in &rows {
            *totals.entry(row.group_key.as_str()).or_default() += &row.aggregate.metrics;
        }
        let mut ranked = totals.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|(left_key, left), (right_key, right)| {
            query
                .rank_by
                .descending(left, right)
                .then_with(|| left_key.cmp(right_key))
        });
        let merged = ranked.len() > query.top;
        let mut keys = ranked
            .iter()
            .take(query.top)
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        if query.group_by.is_none() && keys.is_empty() {
            keys.push(ALL);
        }
        let other = keys.len();
        let width = other + usize::from(merged);
        let column = keys
            .iter()
            .enumerate()
            .map(|(index, key)| (*key, index))
            .collect::<HashMap<_, _>>();
        let bucket_index = starts
            .iter()
            .enumerate()
            .map(|(index, start)| (start.key.as_str(), index))
            .collect::<HashMap<_, _>>();

        let mut cells = vec![UsageMetrics::default(); starts.len() * width];
        for row in &rows {
            let bucket = *bucket_index.get(row.bucket_key.as_str()).ok_or_else(|| {
                DatabaseError::Unreadable {
                    field: "occurred_at",
                    value: row.bucket_key.clone(),
                }
            })?;
            let key = column.get(row.group_key.as_str()).copied().unwrap_or(other);
            cells[bucket * width + key] += &row.aggregate.metrics;
        }

        let group_by = query.group_by;
        let mut names = keys
            .into_iter()
            .map(|key| match group_by {
                Some(dimension) => dimension.key(key.to_owned()),
                None => Ok(ALL.to_owned()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if merged {
            names.push(OTHER.to_owned());
        }

        Ok(starts
            .iter()
            .enumerate()
            .flat_map(|(bucket, start)| {
                names
                    .iter()
                    .enumerate()
                    .map(move |(key, name)| (bucket, start, key, name))
            })
            .map(|(bucket, start, key, name)| Self {
                start: start.start,
                key: name.clone(),
                metrics: cells[bucket * width + key],
            })
            .collect())
    }
}
