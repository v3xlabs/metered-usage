//! One metered request, as it is stored and as it is asked about.

pub mod cliproxy;
pub mod dead_letter;
pub mod ingest;
pub mod litellm;

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteTypeInfo, SqliteValueRef};
use sqlx::{Database as SqlxDatabase, Decode, Encode, FromRow, QueryBuilder, Sqlite, Type};

use crate::account::{Account, AccountSummary, NewAccount};
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::price::ModelPrice;
use crate::source::Source;

/// The account summary reads `provider` as the account's, which can differ from the
/// provider the event was recorded under, so the event's own is read as `event_provider`.
const SELECT: &str = "SELECT usage_event.event_id, usage_event.source_id, \
     usage_event.account_id, usage_event.upstream_id, usage_event.occurred_at, \
     usage_event.provider AS event_provider, usage_event.model, usage_event.model_alias, \
     usage_event.endpoint, usage_event.caller, usage_event.harness, usage_event.user_agent, \
     usage_event.session_id, usage_event.input_tokens, usage_event.output_tokens, \
     usage_event.reasoning_tokens, usage_event.cache_read_tokens, \
     usage_event.cache_write_tokens, usage_event.unclassified_tokens, \
     usage_event.total_tokens, usage_event.token_quality, usage_event.latency_ms, \
     usage_event.ttft_ms, usage_event.streamed, usage_event.status_code, usage_event.failed, \
     usage_event.error_message, usage_event.service_tier, usage_event.reasoning_effort, \
     usage_event.list_cost_usd, usage_event.billed_cost_usd, \
     source.name AS source_name, account.provider, account.auth_kind, account.label, \
     account.display_name \
     FROM usage_event \
     JOIN account ON account.account_id = usage_event.account_id \
     JOIN source ON source.source_id = usage_event.source_id";

#[derive(Debug, Clone, FromRow)]
pub struct UsageEvent {
    #[sqlx(rename = "event_id")]
    pub id: Id<UsageEvent>,
    pub source_id: Id<Source>,
    #[sqlx(flatten)]
    pub account: AccountSummary,
    pub upstream_id: String,
    #[sqlx(try_from = "StoredTimestamp")]
    pub occurred_at: Timestamp,
    #[sqlx(rename = "event_provider")]
    pub provider: String,
    pub model: String,
    pub model_alias: Option<String>,
    pub endpoint: String,
    pub caller: Option<String>,
    pub harness: Option<String>,
    pub user_agent: Option<String>,
    pub session_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub unclassified_tokens: i64,
    pub total_tokens: i64,
    pub token_quality: Option<TokenQuality>,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub streamed: bool,
    pub status_code: i64,
    pub failed: bool,
    pub error_message: Option<String>,
    pub service_tier: Option<String>,
    pub reasoning_effort: Option<String>,
    pub list_cost_usd: Option<f64>,
    pub billed_cost_usd: Option<f64>,
}

impl UsageEvent {
    /// Oldest first.
    pub async fn load_all(
        database: &Database,
        ids: &[Id<UsageEvent>],
    ) -> Result<Vec<Self>, DatabaseError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut builder = QueryBuilder::<Sqlite>::new(SELECT);
        builder.push(" WHERE usage_event.event_id IN (");
        let mut separated = builder.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        builder.push(") ORDER BY usage_event.event_id");

        Ok(builder
            .build_query_as::<Self>()
            .fetch_all(&database.pool)
            .await?)
    }

    /// Newest first, or oldest first after a [`Cursor::After`].
    pub async fn page(
        database: &Database,
        filter: &UsageFilter,
        limit: i64,
        cursor: Option<Cursor>,
    ) -> Result<Vec<Self>, DatabaseError> {
        let mut builder = QueryBuilder::<Sqlite>::new(SELECT);
        filter.push_to(&mut builder);
        let order = match cursor {
            None => " ORDER BY usage_event.event_id DESC",
            Some(Cursor::Before(before)) => {
                builder
                    .push(" AND usage_event.event_id < ")
                    .push_bind(before);
                " ORDER BY usage_event.event_id DESC"
            }
            Some(Cursor::After(after)) => {
                builder
                    .push(" AND usage_event.event_id > ")
                    .push_bind(after);
                " ORDER BY usage_event.event_id"
            }
        };
        builder.push(order).push(" LIMIT ").push_bind(limit);

        Ok(builder
            .build_query_as::<Self>()
            .fetch_all(&database.pool)
            .await?)
    }
}

/// Where a page starts, as the id of an event the client already has.
#[derive(Debug, Clone, Copy)]
pub enum Cursor {
    Before(Id<UsageEvent>),
    After(Id<UsageEvent>),
}

/// What the live usage stream carries: an event as ingest committed it, or the cost a
/// later price fill committed for one.
#[derive(Debug, Clone)]
pub enum UsageUpdate {
    Recorded(Box<UsageEvent>),
    Priced(PricedEvent),
}

/// The cost a fill froze onto an event.
#[derive(Debug, Clone, Copy, FromRow)]
pub struct PricedEvent {
    pub event_id: Id<UsageEvent>,
    pub price_id: Id<ModelPrice>,
    pub list_cost_usd: f64,
    pub billed_cost_usd: f64,
}

/// A record as the gateway described it, before this service gave it an identity.
#[derive(Debug)]
pub struct NewUsageEvent {
    pub upstream_id: String,
    pub occurred_at: Timestamp,
    pub provider: String,
    pub model: String,
    pub model_alias: Option<String>,
    pub endpoint: String,
    pub account: NewAccount,
    /// The client credential the request arrived with, masked.
    pub caller: Option<String>,
    pub harness: Option<String>,
    pub user_agent: Option<String>,
    pub session_id: Option<String>,
    /// Every input token, cache reads and writes included.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub unclassified_tokens: i64,
    pub total_tokens: i64,
    pub token_quality: Option<TokenQuality>,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub streamed: bool,
    pub status_code: i64,
    pub failed: bool,
    pub error_message: Option<String>,
    pub service_tier: Option<String>,
    pub reasoning_effort: Option<String>,
    /// What the gateway itself charged, when it says.
    pub billed_cost_usd: Option<f64>,
}

/// The gateway's own verdict on how its input and output counts partition the total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenQuality {
    Complete,
    Unclassified,
    Inconsistent,
}

impl TokenQuality {
    #[must_use]
    pub fn stored(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Unclassified => "unclassified",
            Self::Inconsistent => "inconsistent",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a token quality")]
pub struct UnknownTokenQuality(pub String);

impl FromStr for TokenQuality {
    type Err = UnknownTokenQuality;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "complete" => Ok(Self::Complete),
            "unclassified" => Ok(Self::Unclassified),
            "inconsistent" => Ok(Self::Inconsistent),
            other => Err(UnknownTokenQuality(other.to_owned())),
        }
    }
}

impl fmt::Display for TokenQuality {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stored())
    }
}

impl Type<Sqlite> for TokenQuality {
    fn type_info() -> SqliteTypeInfo {
        <str as Type<Sqlite>>::type_info()
    }

    fn compatible(info: &SqliteTypeInfo) -> bool {
        <str as Type<Sqlite>>::compatible(info)
    }
}

impl<'r> Decode<'r, Sqlite> for TokenQuality {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(<&str as Decode<'r, Sqlite>>::decode(value)?.parse()?)
    }
}

impl<'q> Encode<'q, Sqlite> for TokenQuality {
    fn encode_by_ref(
        &self,
        buffer: &mut <Sqlite as SqlxDatabase>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <&str as Encode<'q, Sqlite>>::encode(self.stored(), buffer)
    }
}

#[derive(Debug, Default)]
pub struct UsageFilter {
    pub from: Option<Timestamp>,
    pub to: Option<Timestamp>,
    pub source_id: Option<Id<Source>>,
    pub model: Option<String>,
    pub account_id: Option<Id<Account>>,
    pub failed: Option<bool>,
}

impl UsageFilter {
    /// Always leaves the statement in a state another `AND` can be appended to. Every
    /// column is qualified, because a page joins tables that share column names.
    fn push_to(&self, builder: &mut QueryBuilder<Sqlite>) {
        builder.push(" WHERE 1 = 1");
        if let Some(from) = self.from {
            builder
                .push(" AND usage_event.occurred_at >= ")
                .push_bind(StoredTimestamp::from(from));
        }
        if let Some(to) = self.to {
            builder
                .push(" AND usage_event.occurred_at <= ")
                .push_bind(StoredTimestamp::from(to));
        }
        if let Some(source_id) = self.source_id {
            builder
                .push(" AND usage_event.source_id = ")
                .push_bind(source_id);
        }
        if let Some(model) = self.model.as_deref() {
            builder.push(" AND usage_event.model = ").push_bind(model);
        }
        if let Some(account_id) = self.account_id {
            builder
                .push(" AND usage_event.account_id = ")
                .push_bind(account_id);
        }
        if let Some(failed) = self.failed {
            builder.push(" AND usage_event.failed = ").push_bind(failed);
        }
    }
}
