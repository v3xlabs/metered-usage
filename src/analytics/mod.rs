//! Aggregates of metered requests over a window, for dashboards and agents.
//!
//! Every answer is built from one filter, which renders the one `WHERE` clause, and one
//! list of measures, [`metrics::METRICS`], so two answers differ only in what they group by.

pub mod breakdown;
pub mod bucket;
pub mod dimension;
pub mod filter;
pub mod metrics;
pub mod query;
pub mod series;
pub mod session;
pub mod summary;

use sqlx::{QueryBuilder, Sqlite};

use crate::prelude::*;

/// A window this many buckets long is refused rather than zero filled.
pub const MAX_BUCKETS: usize = 5000;

/// Why an analytics question could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum AnalyticsError {
    #[error("account_id {0} names no account")]
    UnknownAccount(Id<Account>),
    #[error("source_id {0} names no source")]
    UnknownSource(Id<Source>),
    #[error(
        "the window holds more than {} buckets; choose a wider bucket or a shorter window",
        MAX_BUCKETS
    )]
    TooManyBuckets,
    #[error("the window is outside the supported range of dates: {0}")]
    Window(#[from] jiff::Error),
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

impl From<sqlx::Error> for AnalyticsError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.into())
    }
}

/// The accounts named, as a usage event describes one, ordered for a picker.
async fn account_summaries(
    database: &Database,
    ids: impl IntoIterator<Item = Id<Account>>,
) -> Result<Vec<AccountSummary>, AnalyticsError> {
    let mut ids = ids.into_iter().peekable();
    if ids.peek().is_none() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT account.account_id, account.source_id, source.name AS source_name, \
         account.provider, account.auth_kind, account.label, account.display_name \
         FROM account JOIN source ON source.source_id = account.source_id \
         WHERE account.account_id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    builder.push(
        ") ORDER BY source.name, account.provider, \
         COALESCE(account.display_name, account.label), account.account_id",
    );

    Ok(builder
        .build_query_as::<AccountSummary>()
        .fetch_all(&database.pool)
        .await?)
}
