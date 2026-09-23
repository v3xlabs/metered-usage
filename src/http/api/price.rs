use std::sync::Arc;

use jiff::Timestamp;
use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};

use crate::app::AppState;
use crate::database::DatabaseError;
use crate::http::api::Error;
use crate::price::catalog::{self, SyncReport};
use crate::price::{ModelPrice, NewPrice, Origin, PriceFilter, Rates};

pub struct PriceApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl PriceApi {
    /// Without `history`, only the latest price of each provider, model, service tier,
    /// threshold and origin.
    #[oai(path = "/prices", method = "get", operation_id = "list_prices")]
    async fn list_prices(
        &self,
        model: Query<Option<String>>,
        origin: Query<Option<OriginOutput>>,
        history: Query<Option<bool>>,
    ) -> PriceListResponse {
        let filter = PriceFilter {
            model: model.0,
            origin: origin.0.map(Origin::from),
            history: history.0.unwrap_or(false),
        };
        match ModelPrice::list(&self.state.database, &filter).await {
            Ok(prices) => PriceListResponse::Found(Json(PriceList {
                prices: prices.into_iter().map(ModelPriceOutput::from).collect(),
            })),
            Err(error) => PriceListResponse::Failed(Json(failure("list_prices", &error))),
        }
    }

    /// Adds a manual price. A price is never edited: a change is a new row with a later
    /// `effective_from`, which defaults to now.
    #[oai(path = "/prices", method = "post", operation_id = "create_price")]
    async fn create_price(&self, input: Json<PriceInput>) -> PriceResponse {
        let price = match NewPrice::try_from(input.0) {
            Ok(price) => price,
            Err(error) => return PriceResponse::Invalid(Json(error)),
        };
        match ModelPrice::insert(&self.state.database, &price, Origin::Manual).await {
            Ok(price) => PriceResponse::Created(Json(price.into())),
            Err(error) => PriceResponse::Failed(Json(failure("create_price", &error))),
        }
    }

    /// Reads every configured LiteLLM map now. A map that cannot be read is named in
    /// `failed_sources` and the others still land.
    #[oai(path = "/prices/sync", method = "post", operation_id = "sync_prices")]
    async fn sync_prices(&self) -> SyncResponse {
        match catalog::sync(&self.state).await {
            Ok(report) => SyncResponse::Synced(Json(report.into())),
            Err(error) => SyncResponse::Failed(Json(failure("sync_prices", &error))),
        }
    }

    /// Prices every event in the range again with the prices in force when it happened.
    /// Both bounds are inclusive and optional.
    #[oai(path = "/prices/reprice", method = "post", operation_id = "reprice")]
    async fn reprice(&self, input: Json<RepriceInput>) -> RepriceResponse {
        let (from, to) = match (
            instant(input.0.from.as_deref(), "from"),
            instant(input.0.to.as_deref(), "to"),
        ) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(error), _) | (_, Err(error)) => return RepriceResponse::Invalid(Json(error)),
        };
        match ModelPrice::reprice(&self.state, from, to).await {
            Ok(repriced) => RepriceResponse::Repriced(Json(Repriced {
                repriced: i64::try_from(repriced).unwrap_or(i64::MAX),
            })),
            Err(error) => RepriceResponse::Failed(Json(failure("reprice", &error))),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "ModelPrice", skip_serializing_if_is_none)]
struct ModelPriceOutput {
    price_id: String,
    provider: Option<String>,
    model: String,
    service_tier: Option<String>,
    min_input_tokens: i64,
    origin: OriginOutput,
    effective_from: String,
    input_usd_per_mtok: f64,
    cached_input_usd_per_mtok: f64,
    cache_write_usd_per_mtok: f64,
    output_usd_per_mtok: f64,
}

impl From<ModelPrice> for ModelPriceOutput {
    fn from(price: ModelPrice) -> Self {
        Self {
            price_id: price.id.encode(),
            provider: price.provider,
            model: price.model,
            service_tier: price.service_tier,
            min_input_tokens: price.min_input_tokens,
            origin: price.origin.into(),
            effective_from: price.effective_from.to_string(),
            input_usd_per_mtok: price.rates.input_usd_per_mtok,
            cached_input_usd_per_mtok: price.rates.cached_input_usd_per_mtok,
            cache_write_usd_per_mtok: price.rates.cache_write_usd_per_mtok,
            output_usd_per_mtok: price.rates.output_usd_per_mtok,
        }
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "PriceOrigin")]
enum OriginOutput {
    #[oai(rename = "manual")]
    Manual,
    #[oai(rename = "litellm_live")]
    LiteLlmLive,
    #[oai(rename = "litellm_public")]
    LiteLlmPublic,
}

impl From<Origin> for OriginOutput {
    fn from(origin: Origin) -> Self {
        match origin {
            Origin::Manual => Self::Manual,
            Origin::LiteLlmLive => Self::LiteLlmLive,
            Origin::LiteLlmPublic => Self::LiteLlmPublic,
        }
    }
}

impl From<OriginOutput> for Origin {
    fn from(origin: OriginOutput) -> Self {
        match origin {
            OriginOutput::Manual => Self::Manual,
            OriginOutput::LiteLlmLive => Self::LiteLlmLive,
            OriginOutput::LiteLlmPublic => Self::LiteLlmPublic,
        }
    }
}

#[derive(Debug, Object)]
struct PriceList {
    prices: Vec<ModelPriceOutput>,
}

/// `provider` and `service_tier` absent mean any. `min_input_tokens` is the input size the
/// price starts at, 0 by default.
#[derive(Debug, Object)]
struct PriceInput {
    provider: Option<String>,
    model: String,
    service_tier: Option<String>,
    min_input_tokens: Option<i64>,
    effective_from: Option<String>,
    input_usd_per_mtok: f64,
    cached_input_usd_per_mtok: f64,
    cache_write_usd_per_mtok: f64,
    output_usd_per_mtok: f64,
}

impl TryFrom<PriceInput> for NewPrice {
    type Error = Error;

    fn try_from(input: PriceInput) -> Result<Self, Self::Error> {
        let model = input.model.trim();
        if model.is_empty() {
            return Err(invalid("model must not be blank"));
        }
        let min_input_tokens = input.min_input_tokens.unwrap_or(0);
        if min_input_tokens < 0 {
            return Err(invalid("min_input_tokens must not be negative"));
        }
        let rates = Rates {
            input_usd_per_mtok: input.input_usd_per_mtok,
            cached_input_usd_per_mtok: input.cached_input_usd_per_mtok,
            cache_write_usd_per_mtok: input.cache_write_usd_per_mtok,
            output_usd_per_mtok: input.output_usd_per_mtok,
        };
        if [
            rates.input_usd_per_mtok,
            rates.cached_input_usd_per_mtok,
            rates.cache_write_usd_per_mtok,
            rates.output_usd_per_mtok,
        ]
        .iter()
        .any(|rate| !rate.is_finite() || *rate < 0.0)
        {
            return Err(invalid("every price must be a number no less than 0"));
        }

        Ok(Self {
            provider: blank_to_none(input.provider),
            model: model.to_owned(),
            service_tier: blank_to_none(input.service_tier),
            min_input_tokens,
            effective_from: instant(input.effective_from.as_deref(), "effective_from")?
                .unwrap_or_else(Timestamp::now),
            rates,
        })
    }
}

#[derive(Debug, Object)]
struct SyncOutput {
    inserted: i64,
    unchanged: i64,
    /// The origin of every map that could not be read.
    failed_sources: Vec<String>,
}

impl From<SyncReport> for SyncOutput {
    fn from(report: SyncReport) -> Self {
        Self {
            inserted: report.inserted,
            unchanged: report.unchanged,
            failed_sources: report.failed_sources,
        }
    }
}

#[derive(Debug, Object)]
struct RepriceInput {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Object)]
struct Repriced {
    repriced: i64,
}

#[derive(ApiResponse)]
enum PriceListResponse {
    #[oai(status = 200)]
    Found(Json<PriceList>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum PriceResponse {
    #[oai(status = 200)]
    Created(Json<ModelPriceOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum SyncResponse {
    #[oai(status = 200)]
    Synced(Json<SyncOutput>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum RepriceResponse {
    #[oai(status = 200)]
    Repriced(Json<Repriced>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn instant(value: Option<&str>, name: &'static str) -> Result<Option<Timestamp>, Error> {
    value
        .map(str::parse::<Timestamp>)
        .transpose()
        .map_err(|_| invalid(&format!("{name} must be an RFC 3339 timestamp")))
}

fn invalid(message: &str) -> Error {
    Error {
        message: message.to_owned(),
    }
}

fn failure(operation: &'static str, error: &DatabaseError) -> Error {
    tracing::error!(%operation, %error, "price operation failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
