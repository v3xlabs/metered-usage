//! What a model's tokens cost at list rates, and the history of that rate.

pub mod catalog;
pub mod cost;

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteTypeInfo, SqliteValueRef};
use sqlx::{Database as SqlxDatabase, Decode, Encode, FromRow, QueryBuilder, Sqlite, Type};
use tokio::time::MissedTickBehavior;

use crate::app::AppState;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;

const COLUMNS: &str = "price_id, provider, model, service_tier, min_input_tokens, origin, \
     effective_from, input_usd_per_mtok, cached_input_usd_per_mtok, cache_write_usd_per_mtok, \
     output_usd_per_mtok";

const FILL_INTERVAL: Duration = Duration::from_mins(1);
const SECONDS_PER_HOUR: u64 = 3600;

#[derive(Debug, Clone, FromRow)]
pub struct ModelPrice {
    #[sqlx(rename = "price_id")]
    pub id: Id<ModelPrice>,
    pub provider: Option<String>,
    pub model: String,
    pub service_tier: Option<String>,
    pub min_input_tokens: i64,
    pub origin: Origin,
    #[sqlx(try_from = "StoredTimestamp")]
    pub effective_from: Timestamp,
    #[sqlx(flatten)]
    pub rates: Rates,
}

impl ModelPrice {
    /// Without `history`, only the latest row of each (provider, model, service tier,
    /// threshold, origin), which is the rate a new request would be charged.
    pub async fn list(
        database: &Database,
        filter: &PriceFilter,
    ) -> Result<Vec<Self>, DatabaseError> {
        let mut builder =
            QueryBuilder::<Sqlite>::new(format!("SELECT {COLUMNS} FROM model_price WHERE 1 = 1"));
        if let Some(model) = &filter.model {
            builder.push(" AND model = ").push_bind(model);
        }
        if let Some(origin) = filter.origin {
            builder.push(" AND origin = ").push_bind(origin);
        }
        if !filter.history {
            builder.push(
                " AND NOT EXISTS (SELECT 1 FROM model_price AS later \
                 WHERE later.model = model_price.model \
                 AND later.provider IS model_price.provider \
                 AND later.service_tier IS model_price.service_tier \
                 AND later.min_input_tokens = model_price.min_input_tokens \
                 AND later.origin = model_price.origin \
                 AND (later.effective_from > model_price.effective_from \
                 OR (later.effective_from = model_price.effective_from \
                 AND later.price_id > model_price.price_id)))",
            );
        }
        builder.push(
            " ORDER BY model, provider, service_tier, min_input_tokens, origin, \
             effective_from DESC, price_id DESC",
        );

        Ok(builder
            .build_query_as::<Self>()
            .fetch_all(&database.pool)
            .await?)
    }

    pub async fn insert(
        database: &Database,
        price: &NewPrice,
        origin: Origin,
    ) -> Result<Self, DatabaseError> {
        let id = database.ids.next::<Self>();
        sqlx::query(
            "INSERT INTO model_price (price_id, provider, model, service_tier, min_input_tokens, \
             origin, effective_from, input_usd_per_mtok, cached_input_usd_per_mtok, \
             cache_write_usd_per_mtok, output_usd_per_mtok, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&price.provider)
        .bind(&price.model)
        .bind(&price.service_tier)
        .bind(price.min_input_tokens)
        .bind(origin)
        .bind(StoredTimestamp::from(price.effective_from))
        .bind(price.rates.input_usd_per_mtok)
        .bind(price.rates.cached_input_usd_per_mtok)
        .bind(price.rates.cache_write_usd_per_mtok)
        .bind(price.rates.output_usd_per_mtok)
        .bind(StoredTimestamp::from(Timestamp::now()))
        .execute(&database.pool)
        .await?;

        Ok(Self {
            id,
            provider: price.provider.clone(),
            model: price.model.clone(),
            service_tier: price.service_tier.clone(),
            min_input_tokens: price.min_input_tokens,
            origin,
            effective_from: price.effective_from,
            rates: price.rates,
        })
    }
}

/// US dollars per million tokens of each kind.
#[derive(Debug, Clone, Copy, PartialEq, FromRow)]
pub struct Rates {
    pub input_usd_per_mtok: f64,
    pub cached_input_usd_per_mtok: f64,
    pub cache_write_usd_per_mtok: f64,
    pub output_usd_per_mtok: f64,
}

#[derive(Debug)]
pub struct NewPrice {
    pub provider: Option<String>,
    pub model: String,
    pub service_tier: Option<String>,
    pub min_input_tokens: i64,
    pub effective_from: Timestamp,
    pub rates: Rates,
}

#[derive(Debug, Default)]
pub struct PriceFilter {
    pub model: Option<String>,
    pub origin: Option<Origin>,
    pub history: bool,
}

/// Where a price came from. The order of the variants is their rank: a price from an
/// earlier variant wins over one from a later variant for the same request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Manual,
    LiteLlmLive,
    LiteLlmPublic,
}

impl Origin {
    #[must_use]
    pub fn stored(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::LiteLlmLive => "litellm_live",
            Self::LiteLlmPublic => "litellm_public",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a price origin")]
pub struct UnknownOrigin(pub String);

impl FromStr for Origin {
    type Err = UnknownOrigin;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "manual" => Ok(Self::Manual),
            "litellm_live" => Ok(Self::LiteLlmLive),
            "litellm_public" => Ok(Self::LiteLlmPublic),
            other => Err(UnknownOrigin(other.to_owned())),
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stored())
    }
}

impl Type<Sqlite> for Origin {
    fn type_info() -> SqliteTypeInfo {
        <str as Type<Sqlite>>::type_info()
    }

    fn compatible(info: &SqliteTypeInfo) -> bool {
        <str as Type<Sqlite>>::compatible(info)
    }
}

impl<'r> Decode<'r, Sqlite> for Origin {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(<&str as Decode<'r, Sqlite>>::decode(value)?.parse()?)
    }
}

impl<'q> Encode<'q, Sqlite> for Origin {
    fn encode_by_ref(
        &self,
        buffer: &mut <Sqlite as SqlxDatabase>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <&str as Encode<'q, Sqlite>>::encode(self.stored(), buffer)
    }
}

/// Syncs the catalog now and every `pricing.refresh_hours`, and prices new events every
/// minute.
pub fn spawn(state: Arc<AppState>) {
    // A zero refresh would make the interval panic; an hour is the shortest that is sane
    // for a file that changes a few times a week.
    let refresh = Duration::from_secs(state.config.pricing.refresh_hours.max(1) * SECONDS_PER_HOUR);
    let syncing = Arc::clone(&state);
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(refresh);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            match catalog::sync(&syncing).await {
                Ok(report) => tracing::info!(
                    inserted = report.inserted,
                    unchanged = report.unchanged,
                    failed = ?report.failed_sources,
                    "price catalog synced"
                ),
                Err(error) => tracing::error!(%error, "price catalog sync failed"),
            }
        }
    });
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(FILL_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if let Err(error) = ModelPrice::fill(&state.database).await {
                tracing::error!(%error, "pricing events failed");
            }
        }
    });
}
