//! What usage can be grouped by, and which values of each a window holds.

use std::collections::{BTreeSet, HashMap};

use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::{AnalyticsError, account_summaries};
use crate::prelude::*;

/// Every event has a value for every dimension: an event that names no harness or no
/// session is grouped under the key `unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Model,
    Provider,
    Account,
    Harness,
    Source,
    Session,
}

impl Dimension {
    /// The key as text, whatever the column holds. Every arm is a literal of this crate.
    #[must_use]
    pub fn expression(self) -> &'static str {
        match self {
            Self::Model => "usage_event.model",
            Self::Provider => "usage_event.provider",
            Self::Account => "CAST(usage_event.account_id AS TEXT)",
            Self::Harness => "COALESCE(usage_event.harness, 'unknown')",
            Self::Source => "CAST(usage_event.source_id AS TEXT)",
            Self::Session => "COALESCE(usage_event.session_id, 'unknown')",
        }
    }

    /// The key as the API spells it: an account or a source by its encoded id.
    pub fn key(self, stored: String) -> Result<String, DatabaseError> {
        match self {
            Self::Account => stored_id::<Account>(&stored, "account_id").map(Id::encode),
            Self::Source => stored_id::<Source>(&stored, "source_id").map(Id::encode),
            Self::Model | Self::Provider | Self::Harness | Self::Session => Ok(stored),
        }
    }

    /// What a reader calls each stored key: an account by its display name, else its label,
    /// and a source by its name. Keys of the other dimensions already read as names, so they
    /// get none, and neither does an account that has neither name.
    pub async fn labels(
        self,
        database: &Database,
        stored: impl IntoIterator<Item = &str>,
    ) -> Result<HashMap<String, String>, AnalyticsError> {
        match self {
            Self::Account => {
                let ids = stored
                    .into_iter()
                    .map(|key| stored_id::<Account>(key, "account_id"))
                    .collect::<Result<BTreeSet<_>, _>>()?;
                Ok(account_summaries(database, ids)
                    .await?
                    .into_iter()
                    .filter_map(|account| {
                        let name = account.display_name.or(account.label)?;
                        Some((account.id.raw().to_string(), name))
                    })
                    .collect())
            }
            Self::Source => {
                let ids = stored
                    .into_iter()
                    .map(|key| stored_id::<Source>(key, "source_id"))
                    .collect::<Result<BTreeSet<_>, _>>()?;
                Ok(SourceRef::load_all(database, ids)
                    .await?
                    .into_iter()
                    .map(|source| (source.id.raw().to_string(), source.name))
                    .collect())
            }
            Self::Model | Self::Provider | Self::Harness | Self::Session => Ok(HashMap::new()),
        }
    }
}

/// The values of each filterable dimension that at least one event in the window has.
#[derive(Debug)]
pub struct Dimensions {
    pub models: Vec<String>,
    pub providers: Vec<String>,
    pub harnesses: Vec<String>,
    pub accounts: Vec<AccountSummary>,
    pub sources: Vec<SourceRef>,
}

impl Dimensions {
    pub async fn load(
        database: &Database,
        filter: &AnalyticsFilter,
    ) -> Result<Self, AnalyticsError> {
        filter.check(database).await?;
        let mut builder =
            QueryBuilder::<Sqlite>::new("SELECT usage_event.model, usage_event.provider, ");
        builder
            .push(Dimension::Harness.expression())
            .push(" AS harness, usage_event.account_id, usage_event.source_id FROM usage_event");
        filter.push_where(&mut builder);
        builder.push(" GROUP BY 1, 2, 3, 4, 5");
        let rows = builder
            .build_query_as::<Combination>()
            .fetch_all(&database.pool)
            .await?;

        let mut models = BTreeSet::new();
        let mut providers = BTreeSet::new();
        let mut harnesses = BTreeSet::new();
        let mut account_ids = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        for row in rows {
            models.insert(row.model);
            providers.insert(row.provider);
            harnesses.insert(row.harness);
            account_ids.insert(row.account_id);
            source_ids.insert(row.source_id);
        }

        Ok(Self {
            models: models.into_iter().collect(),
            providers: providers.into_iter().collect(),
            harnesses: harnesses.into_iter().collect(),
            accounts: account_summaries(database, account_ids).await?,
            sources: SourceRef::load_all(database, source_ids).await?,
        })
    }
}

/// A source as a filter picker names it.
#[derive(Debug, Clone, FromRow)]
pub struct SourceRef {
    #[sqlx(rename = "source_id")]
    pub id: Id<Source>,
    pub name: String,
    pub kind: SourceKind,
}

impl SourceRef {
    async fn load_all(
        database: &Database,
        ids: BTreeSet<Id<Source>>,
    ) -> Result<Vec<Self>, AnalyticsError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT source_id, name, kind FROM source WHERE source_id IN (",
        );
        let mut separated = builder.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
        builder.push(") ORDER BY name, source_id");

        Ok(builder
            .build_query_as::<Self>()
            .fetch_all(&database.pool)
            .await?)
    }
}

#[derive(FromRow)]
struct Combination {
    model: String,
    provider: String,
    harness: String,
    account_id: Id<Account>,
    source_id: Id<Source>,
}

fn stored_id<T>(stored: &str, field: &'static str) -> Result<Id<T>, DatabaseError> {
    stored
        .parse::<i64>()
        .map(Id::from_raw)
        .map_err(|_| DatabaseError::Unreadable {
            field,
            value: stored.to_owned(),
        })
}
