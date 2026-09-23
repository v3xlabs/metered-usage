//! Usage per agent session, as the harness named it.

use std::collections::{BTreeSet, HashMap};

use jiff::Timestamp;
use sqlx::FromRow;

use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::metrics::{RankBy, UsageMetrics};
use crate::analytics::query::{Aggregate, Column, aggregate};
use crate::analytics::{AnalyticsError, account_summaries};
use crate::prelude::*;

/// A session that moved between accounts, sources or harnesses is reported under the one
/// that carried most of its requests.
#[derive(Debug)]
pub struct SessionUsage {
    pub session_id: String,
    pub source_id: Id<Source>,
    pub account: AccountSummary,
    pub harness: Option<String>,
    /// Sorted.
    pub models: Vec<String>,
    pub first_at: Timestamp,
    pub last_at: Timestamp,
    pub metrics: UsageMetrics,
}

#[derive(FromRow)]
struct SessionRow {
    session_id: String,
    account_id: Id<Account>,
    source_id: Id<Source>,
    harness: Option<String>,
    model: String,
    #[sqlx(flatten)]
    aggregate: Aggregate,
}

struct Tally {
    aggregate: Aggregate,
    accounts: HashMap<Id<Account>, i64>,
    sources: HashMap<Id<Source>, i64>,
    harnesses: HashMap<String, i64>,
    models: BTreeSet<String>,
}

struct Chosen {
    session_id: String,
    account_id: Option<Id<Account>>,
    source_id: Option<Id<Source>>,
    harness: Option<String>,
    models: Vec<String>,
    aggregate: Aggregate,
}

impl SessionUsage {
    /// Events without a session are left out.
    pub async fn list(
        database: &Database,
        filter: &AnalyticsFilter,
        rank_by: RankBy,
        limit: usize,
    ) -> Result<Vec<Self>, AnalyticsError> {
        filter.check(database).await?;
        let rows = aggregate(
            filter,
            &[
                (Column::Expression("usage_event.session_id"), "session_id"),
                (Column::Expression("usage_event.account_id"), "account_id"),
                (Column::Expression("usage_event.source_id"), "source_id"),
                (Column::Expression("usage_event.harness"), "harness"),
                (Column::Expression("usage_event.model"), "model"),
            ],
            Some("usage_event.session_id IS NOT NULL"),
        )
        .build_query_as::<SessionRow>()
        .fetch_all(&database.pool)
        .await?;

        let mut sessions = HashMap::<String, Tally>::new();
        for row in rows {
            let requests = row.aggregate.metrics.requests;
            let tally = match sessions.entry(row.session_id) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let tally = entry.into_mut();
                    tally.aggregate += &row.aggregate;
                    tally
                }
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(Tally {
                    aggregate: row.aggregate,
                    accounts: HashMap::new(),
                    sources: HashMap::new(),
                    harnesses: HashMap::new(),
                    models: BTreeSet::new(),
                }),
            };
            *tally.accounts.entry(row.account_id).or_default() += requests;
            *tally.sources.entry(row.source_id).or_default() += requests;
            if let Some(harness) = row.harness {
                *tally.harnesses.entry(harness).or_default() += requests;
            }
            tally.models.insert(row.model);
        }

        let mut ranked = sessions.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|(left_id, left), (right_id, right)| {
            rank_by
                .descending(&left.aggregate.metrics, &right.aggregate.metrics)
                .then_with(|| left_id.cmp(right_id))
        });
        ranked.truncate(limit);

        let chosen = ranked
            .into_iter()
            .map(|(session_id, tally)| Chosen {
                session_id,
                account_id: leader(tally.accounts),
                source_id: leader(tally.sources),
                harness: leader(tally.harnesses),
                models: tally.models.into_iter().collect(),
                aggregate: tally.aggregate,
            })
            .collect::<Vec<_>>();
        let accounts = account_summaries(
            database,
            chosen
                .iter()
                .filter_map(|session| session.account_id)
                .collect::<BTreeSet<_>>(),
        )
        .await?
        .into_iter()
        .map(|account| (account.id, account))
        .collect::<HashMap<_, _>>();

        chosen
            .into_iter()
            .map(|session| {
                let unreadable = || DatabaseError::Unreadable {
                    field: "session_id",
                    value: session.session_id.clone(),
                };
                let account = session
                    .account_id
                    .and_then(|id| accounts.get(&id))
                    .cloned()
                    .ok_or_else(unreadable)?;
                let source_id = session.source_id.ok_or_else(unreadable)?;

                Ok(Self {
                    source_id,
                    account,
                    harness: session.harness,
                    models: session.models,
                    first_at: session.aggregate.first_at,
                    last_at: session.aggregate.last_at,
                    metrics: session.aggregate.metrics,
                    session_id: session.session_id,
                })
            })
            .collect()
    }
}

/// The value with the most requests, the least value among equals.
fn leader<K: Ord>(counts: HashMap<K, i64>) -> Option<K> {
    counts
        .into_iter()
        .max_by(|(left, left_count), (right, right_count)| {
            left_count.cmp(right_count).then_with(|| right.cmp(left))
        })
        .map(|(key, _)| key)
}
