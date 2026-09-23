//! Usage split by one dimension, largest group first.

use sqlx::FromRow;

use crate::analytics::AnalyticsError;
use crate::analytics::dimension::Dimension;
use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::metrics::{RankBy, UsageMetrics};
use crate::analytics::query::{Aggregate, Column, aggregate};
use crate::prelude::*;

#[derive(Debug)]
pub struct Breakdown {
    pub rows: Vec<BreakdownRow>,
    /// Every group, including those cut by the limit.
    pub total: UsageMetrics,
}

#[derive(Debug)]
pub struct BreakdownRow {
    pub key: String,
    pub metrics: UsageMetrics,
}

#[derive(FromRow)]
struct GroupRow {
    group_key: String,
    #[sqlx(flatten)]
    aggregate: Aggregate,
}

impl Breakdown {
    pub async fn load(
        database: &Database,
        filter: &AnalyticsFilter,
        group_by: Dimension,
        rank_by: RankBy,
        limit: usize,
    ) -> Result<Self, AnalyticsError> {
        filter.check(database).await?;
        let mut groups = aggregate(filter, &[(Column::Key(group_by), "group_key")], None)
            .build_query_as::<GroupRow>()
            .fetch_all(&database.pool)
            .await?;

        let mut total = UsageMetrics::default();
        for group in &groups {
            total += &group.aggregate.metrics;
        }
        groups.sort_by(|left, right| {
            rank_by
                .descending(&left.aggregate.metrics, &right.aggregate.metrics)
                .then_with(|| left.group_key.cmp(&right.group_key))
        });
        groups.truncate(limit);
        let rows = groups
            .into_iter()
            .map(|group| {
                Ok(BreakdownRow {
                    key: group_by.key(group.group_key)?,
                    metrics: group.aggregate.metrics,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Self { rows, total })
    }
}
