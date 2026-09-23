//! Which events a question is about, and the one `WHERE` clause that selects them.

use jiff::Timestamp;
use jiff::tz::Offset;
use sqlx::{Encode, QueryBuilder, Sqlite, Type};

use crate::analytics::AnalyticsError;
use crate::analytics::dimension::Dimension;
use crate::database::codec::StoredTimestamp;
use crate::prelude::*;

/// Values of one list are alternatives; lists are all required. An empty list does not
/// filter.
#[derive(Debug, Clone)]
pub struct AnalyticsFilter {
    /// Absent, the window starts at the first matching event.
    pub from: Option<Timestamp>,
    /// Exclusive, and fixed once, so every query of one answer reads the same end.
    pub to: Timestamp,
    /// The viewer's offset from UTC, which places day and week boundaries.
    pub offset: Offset,
    pub source_ids: Vec<Id<Source>>,
    pub account_ids: Vec<Id<Account>>,
    pub models: Vec<String>,
    pub providers: Vec<String>,
    /// Matched against the harness key, so `unknown` selects events that name none.
    pub harnesses: Vec<String>,
}

impl AnalyticsFilter {
    /// Every event in the window, with `to` defaulting to now.
    #[must_use]
    pub fn window(from: Option<Timestamp>, to: Option<Timestamp>, offset: Offset) -> Self {
        Self {
            from,
            to: to.unwrap_or_else(Timestamp::now),
            offset,
            source_ids: Vec::new(),
            account_ids: Vec::new(),
            models: Vec::new(),
            providers: Vec::new(),
            harnesses: Vec::new(),
        }
    }

    /// Refuses an id that names nothing, which would otherwise read as an empty answer.
    pub async fn check(&self, database: &Database) -> Result<(), AnalyticsError> {
        if let Some(id) = missing(database, "source", "source_id", &self.source_ids).await? {
            return Err(AnalyticsError::UnknownSource(id));
        }
        if let Some(id) = missing(database, "account", "account_id", &self.account_ids).await? {
            return Err(AnalyticsError::UnknownAccount(id));
        }

        Ok(())
    }

    /// The SQLite date modifier that turns a stored UTC instant into the viewer's time.
    #[must_use]
    pub fn shift(&self) -> String {
        format!("{:+} seconds", self.offset.seconds())
    }

    /// Leaves the statement in a state another `AND` can be appended to.
    pub fn push_where(&self, builder: &mut QueryBuilder<Sqlite>) {
        builder
            .push(" WHERE usage_event.occurred_at < ")
            .push_bind(StoredTimestamp::from(self.to));
        if let Some(from) = self.from {
            builder
                .push(" AND usage_event.occurred_at >= ")
                .push_bind(StoredTimestamp::from(from));
        }
        push_in(builder, "usage_event.source_id", &self.source_ids);
        push_in(builder, "usage_event.account_id", &self.account_ids);
        push_in(builder, "usage_event.model", &self.models);
        push_in(builder, "usage_event.provider", &self.providers);
        push_in(builder, Dimension::Harness.expression(), &self.harnesses);
    }
}

async fn missing<T>(
    database: &Database,
    table: &'static str,
    column: &'static str,
    ids: &[Id<T>],
) -> Result<Option<Id<T>>, AnalyticsError>
where
    T: 'static,
{
    if ids.is_empty() {
        return Ok(None);
    }
    let mut builder =
        QueryBuilder::<Sqlite>::new(format!("SELECT {column} FROM {table} WHERE {column} IN ("));
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    builder.push(")");
    let found = builder
        .build_query_scalar::<Id<T>>()
        .fetch_all(&database.pool)
        .await?;

    Ok(ids.iter().find(|id| !found.contains(id)).copied())
}

fn push_in<V>(builder: &mut QueryBuilder<Sqlite>, column: &'static str, values: &[V])
where
    for<'v> &'v V: Encode<'v, Sqlite> + Type<Sqlite>,
{
    if values.is_empty() {
        return;
    }
    builder.push(" AND ").push(column).push(" IN (");
    let mut separated = builder.separated(", ");
    for value in values {
        separated.push_bind(value);
    }
    builder.push(")");
}
