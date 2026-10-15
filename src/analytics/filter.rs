//! Which events a question is about, and the one `WHERE` clause that selects them.

use jiff::tz::{Offset, TimeZone};
use jiff::{SignedDuration, Timestamp};
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
    /// The viewer's zone, which places hour, day and week boundaries.
    pub time_zone: TimeZone,
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
    pub fn window(from: Option<Timestamp>, to: Option<Timestamp>, time_zone: TimeZone) -> Self {
        Self {
            from,
            to: to.unwrap_or_else(Timestamp::now),
            time_zone,
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

    /// How the viewer's zone shifted every event the window holds. No event lies outside
    /// what is recorded, so the zone's changes are read for that stretch only.
    #[must_use]
    pub fn shift(&self, recorded: Option<Recorded>) -> Shift {
        let Some(recorded) = recorded else {
            return Shift {
                changes: Vec::new(),
                before: self.time_zone.to_offset(self.to),
            };
        };
        let since = self
            .from
            .map_or(recorded.first, |from| from.max(recorded.first));
        // `preceding` leaves out a change made at its own instant, which the last event takes.
        let until = recorded
            .last
            .checked_add(SignedDuration::from_nanos(1))
            .unwrap_or(recorded.last)
            .min(self.to);
        let changes = self
            .time_zone
            .preceding(until)
            .take_while(|transition| transition.timestamp() > since)
            .map(|transition| (transition.timestamp(), transition.offset()))
            .collect();

        Shift {
            changes,
            before: self.time_zone.to_offset(since),
        }
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

/// The first and the last instant of every stored event, whatever a filter selects.
#[derive(Debug, Clone, Copy)]
pub struct Recorded {
    pub first: Timestamp,
    pub last: Timestamp,
}

impl Recorded {
    /// None while no event is stored.
    pub async fn load(database: &Database) -> Result<Option<Self>, AnalyticsError> {
        let (first, last) =
            sqlx::query_as::<_, (Option<StoredTimestamp>, Option<StoredTimestamp>)>(
                "SELECT MIN(occurred_at), MAX(occurred_at) FROM usage_event",
            )
            .fetch_one(&database.pool)
            .await?;

        Ok(first.zip(last).map(|(first, last)| Self {
            first: first.into(),
            last: last.into(),
        }))
    }
}

/// The offsets a zone used over a window, as the SQLite date modifier that turns a stored
/// UTC instant into local time. A zone without daylight saving changes none.
#[derive(Debug, Clone)]
pub struct Shift {
    /// Latest first: from each instant on the zone used that offset.
    changes: Vec<(Timestamp, Offset)>,
    /// In force before the earliest change.
    before: Offset,
}

impl Shift {
    pub fn push(&self, builder: &mut QueryBuilder<Sqlite>) {
        if self.changes.is_empty() {
            builder.push_bind(modifier(self.before));
            return;
        }
        builder.push("CASE");
        for (at, offset) in &self.changes {
            builder
                .push(" WHEN usage_event.occurred_at >= ")
                .push_bind(StoredTimestamp::from(*at))
                .push(" THEN ")
                .push_bind(modifier(*offset));
        }
        builder
            .push(" ELSE ")
            .push_bind(modifier(self.before))
            .push(" END");
    }
}

fn modifier(offset: Offset) -> String {
    format!("{:+} seconds", offset.seconds())
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
