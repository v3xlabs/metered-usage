//! The one aggregate statement every analytics answer runs, differing only in its groups.

use std::ops::AddAssign;

use jiff::Timestamp;
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::analytics::bucket::Bucket;
use crate::analytics::dimension::Dimension;
use crate::analytics::filter::{AnalyticsFilter, Shift};
use crate::analytics::metrics::{FROM, METRICS, UsageMetrics};
use crate::database::codec::StoredTimestamp;

/// Grouped, so never NULL: a group has at least one event.
const SPAN: &str =
    ", MIN(usage_event.occurred_at) AS first_at, MAX(usage_event.occurred_at) AS last_at";

/// One grouping column. Nothing here is request text: a dimension and a bucket render
/// literals of this crate and bind their values, and an expression is a literal its caller
/// wrote.
#[derive(Debug, Clone, Copy)]
pub enum Column<'a> {
    Key(Dimension),
    Bucket(Bucket, &'a Shift),
    Expression(&'static str),
}

/// The measures of one group and the first and last instant it saw.
#[derive(Debug, Clone, Copy, FromRow)]
pub struct Aggregate {
    #[sqlx(flatten)]
    pub metrics: UsageMetrics,
    #[sqlx(try_from = "StoredTimestamp")]
    pub first_at: Timestamp,
    #[sqlx(try_from = "StoredTimestamp")]
    pub last_at: Timestamp,
}

impl AddAssign<&Self> for Aggregate {
    fn add_assign(&mut self, other: &Self) {
        self.metrics += &other.metrics;
        self.first_at = self.first_at.min(other.first_at);
        self.last_at = self.last_at.max(other.last_at);
    }
}

/// `SELECT <columns>, <measures> FROM … WHERE <filter> [AND <only>] GROUP BY <columns>`,
/// each column named by its alias.
#[must_use]
pub fn aggregate(
    filter: &AnalyticsFilter,
    columns: &[(Column<'_>, &'static str)],
    only: Option<&'static str>,
) -> QueryBuilder<Sqlite> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT ");
    for (column, alias) in columns {
        match column {
            Column::Key(dimension) => {
                builder.push(dimension.expression());
            }
            Column::Bucket(bucket, shift) => bucket.push_expression(&mut builder, shift),
            Column::Expression(expression) => {
                builder.push(expression);
            }
        }
        builder.push(" AS ").push(alias).push(", ");
    }
    builder.push(METRICS).push(SPAN).push(FROM);
    filter.push_where(&mut builder);
    if let Some(only) = only {
        builder.push(" AND ").push(only);
    }
    // By position, because an alias such as `model` also names a column of the joined
    // price, and SQLite refuses the ambiguity.
    for position in 1..=columns.len() {
        builder
            .push(if position == 1 { " GROUP BY " } else { ", " })
            .push(position);
    }

    builder
}
