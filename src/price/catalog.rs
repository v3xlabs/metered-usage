//! LiteLLM's model cost map, as published and as a running proxy serves it.

use std::collections::HashMap;
use std::time::Duration;

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::FromRow;

use crate::app::AppState;
use crate::database::DatabaseError;
use crate::database::codec::StoredTimestamp;
use crate::price::{ModelPrice, Origin, Rates};

const LIVE_MAP_PATH: &str = "/public/litellm_model_cost_map";
const FETCH_TIMEOUT: Duration = Duration::from_mins(1);
/// Documents the file's fields rather than pricing a model.
const SAMPLE_ENTRY: &str = "sample_spec";
const LONG_CONTEXT_PREFIX: &str = "input_cost_per_token_above_";
const LONG_CONTEXT_SUFFIX: &str = "k_tokens";
/// The provider LiteLLM bills the long-context rate from the threshold itself, not the token
/// after it (`_INCLUSIVE_THRESHOLD_PROVIDERS` in `litellm_core_utils/llm_cost_calc/utils.py`).
const INCLUSIVE_THRESHOLD_PROVIDER: &str = "xai";
/// The key suffix a price for a service tier carries, and the tier a request names.
const SERVICE_TIERS: [(&str, Option<&str>); 4] = [
    ("", None),
    ("_priority", Some("priority")),
    ("_flex", Some("flex")),
    ("_batches", Some("batch")),
];
const TOKENS_PER_MILLION: f64 = 1e6;
/// Per-token prices are decimals such as `3e-06`, which times a million is not exactly
/// 3. Rounding to a billionth of a dollar per million keeps a price that did not change
/// equal to its stored self.
const RATE_PRECISION: f64 = 1e9;

/// The epoch, not the sync time, dates the first row of a key: events older than the
/// first sync, such as a backlog drained at startup, would otherwise never find a price.
const BEGINNING: Timestamp = Timestamp::UNIX_EPOCH;

/// One map's prices, one entry per (model, service tier, threshold).
#[derive(Debug, Deserialize)]
#[serde(from = "Map<String, Value>")]
pub struct Catalog {
    pub prices: Vec<CatalogPrice>,
}

#[derive(Debug)]
pub struct CatalogPrice {
    pub provider: Option<String>,
    pub model: String,
    pub service_tier: Option<&'static str>,
    pub min_input_tokens: i64,
    pub rates: Rates,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub inserted: i64,
    pub unchanged: i64,
    /// The origin of every map that could not be read.
    pub failed_sources: Vec<String>,
}

#[derive(FromRow)]
struct KnownPrice {
    provider: Option<String>,
    model: String,
    service_tier: Option<String>,
    min_input_tokens: i64,
    #[sqlx(flatten)]
    rates: Rates,
}

type Key = (Option<String>, String, Option<String>, i64);

impl From<Map<String, Value>> for Catalog {
    fn from(map: Map<String, Value>) -> Self {
        let mut prices = Vec::new();
        for (model, entry) in map {
            if model == SAMPLE_ENTRY {
                continue;
            }
            let Value::Object(entry) = entry else {
                continue;
            };
            let provider = entry
                .get("litellm_provider")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let first = prices.len();
            for (threshold, min_input_tokens) in thresholds(&entry) {
                for (suffix, service_tier) in SERVICE_TIERS {
                    if let Some(rates) = rates(&entry, &threshold, suffix) {
                        prices.push(CatalogPrice {
                            provider: provider.clone(),
                            model: model.clone(),
                            service_tier,
                            min_input_tokens,
                            rates,
                        });
                    }
                }
            }
            for tier in entry
                .get("tiered_pricing")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
            {
                // LiteLLM picks the tier whose range holds `start < input <= end`.
                let Some(min_input_tokens) = tier
                    .get("range")
                    .and_then(Value::as_array)
                    .and_then(|range| range.first())
                    .and_then(tokens)
                    .map(|start| start + i64::from(start > 0))
                else {
                    continue;
                };
                let known = prices[first..].iter().any(|price| {
                    price.service_tier.is_none() && price.min_input_tokens == min_input_tokens
                });
                if let Some(rates) = rates(tier, "", "").filter(|_| !known) {
                    prices.push(CatalogPrice {
                        provider: provider.clone(),
                        model: model.clone(),
                        service_tier: None,
                        min_input_tokens,
                        rates,
                    });
                }
            }
        }

        Self { prices }
    }
}

/// Fetches every configured map and records each price that is new or changed. A map
/// that cannot be read is reported and the others still land.
pub async fn sync(state: &AppState) -> Result<SyncReport, DatabaseError> {
    let pricing = &state.config.pricing;
    let mut maps = vec![(Origin::LiteLlmPublic, pricing.public_map_url.clone())];
    if let Some(source) = pricing
        .litellm_source
        .as_deref()
        .and_then(|key| state.config.source(key))
    {
        maps.push((
            Origin::LiteLlmLive,
            format!("{}{LIVE_MAP_PATH}", source.base_url),
        ));
    }

    let mut report = SyncReport::default();
    for (origin, url) in maps {
        match fetch(&url).await {
            Ok(catalog) => record(state, origin, catalog, &mut report).await?,
            Err(error) => {
                tracing::warn!(%origin, %url, %error, "price map could not be read");
                report.failed_sources.push(origin.stored().to_owned());
            }
        }
    }
    ModelPrice::fill(state).await?;

    Ok(report)
}

async fn fetch(url: &str) -> reqwest::Result<Catalog> {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<Catalog>()
        .await
}

async fn record(
    state: &AppState,
    origin: Origin,
    catalog: Catalog,
    report: &mut SyncReport,
) -> Result<(), DatabaseError> {
    let database = &state.database;
    let now = StoredTimestamp::from(Timestamp::now());
    let mut transaction = database.write().await?;
    let mut latest = HashMap::<Key, Rates>::new();
    let known = sqlx::query_as::<_, KnownPrice>(
        "SELECT provider, model, service_tier, min_input_tokens, input_usd_per_mtok, \
         cached_input_usd_per_mtok, cache_write_usd_per_mtok, output_usd_per_mtok \
         FROM model_price WHERE origin = ? ORDER BY effective_from, price_id",
    )
    .bind(origin)
    .fetch_all(&mut *transaction)
    .await?;
    for price in known {
        latest.insert(
            (
                price.provider,
                price.model,
                price.service_tier,
                price.min_input_tokens,
            ),
            price.rates,
        );
    }

    for price in catalog.prices {
        let key = (
            price.provider,
            price.model,
            price.service_tier.map(str::to_owned),
            price.min_input_tokens,
        );
        let effective_from = match latest.get(&key) {
            Some(rates) if *rates == price.rates => {
                report.unchanged += 1;
                continue;
            }
            Some(_) => now,
            None => StoredTimestamp::from(BEGINNING),
        };
        let (provider, model, service_tier, min_input_tokens) = &key;
        sqlx::query(
            "INSERT INTO model_price (price_id, provider, model, service_tier, min_input_tokens, \
             origin, effective_from, input_usd_per_mtok, cached_input_usd_per_mtok, \
             cache_write_usd_per_mtok, output_usd_per_mtok, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(database.ids.next::<ModelPrice>())
        .bind(provider)
        .bind(model)
        .bind(service_tier)
        .bind(min_input_tokens)
        .bind(origin)
        .bind(effective_from)
        .bind(price.rates.input_usd_per_mtok)
        .bind(price.rates.cached_input_usd_per_mtok)
        .bind(price.rates.cache_write_usd_per_mtok)
        .bind(price.rates.output_usd_per_mtok)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        latest.insert(key, price.rates);
        report.inserted += 1;
    }
    transaction.commit().await?;

    Ok(())
}

/// The key tail and first token count of the base price and of every long-context price
/// the entry names. LiteLLM charges `_above_200k_tokens` from 200 001 input tokens on, and
/// from 200 000 for xAI.
fn thresholds(entry: &Map<String, Value>) -> Vec<(String, i64)> {
    let past_threshold = i64::from(
        entry.get("litellm_provider").and_then(Value::as_str) != Some(INCLUSIVE_THRESHOLD_PROVIDER),
    );
    let mut thresholds = vec![(String::new(), 0)];
    for key in entry.keys() {
        let Some(thousands) = key
            .strip_prefix(LONG_CONTEXT_PREFIX)
            .and_then(|rest| rest.strip_suffix(LONG_CONTEXT_SUFFIX))
            .and_then(|thousands| thousands.parse::<i64>().ok())
        else {
            continue;
        };
        thresholds.push((
            format!("_above_{thousands}{LONG_CONTEXT_SUFFIX}"),
            thousands * 1000 + past_threshold,
        ));
    }
    thresholds
}

/// The rates of one threshold and tier, where the entry names an input price for that
/// tier. A rate the entry leaves out falls back as LiteLLM's `_get_token_base_cost` does
/// (`litellm/litellm_core_utils/llm_cost_calc/utils.py`): to the same rate without the
/// tier, then to the tier's rate below the threshold, then to the base rate, and a cache
/// rate named nowhere is the input rate.
fn rates(entry: &Map<String, Value>, threshold: &str, tier: &str) -> Option<Rates> {
    let tails = [
        format!("{threshold}{tier}"),
        threshold.to_owned(),
        tier.to_owned(),
        String::new(),
    ];
    let named = |name: &str, tail: &str| {
        entry
            .get(&format!("{name}{tail}"))
            .and_then(Value::as_f64)
            .map(per_million)
    };
    let price = |name: &str| tails.iter().find_map(|tail| named(name, tail));
    if !tier.is_empty()
        && named("input_cost_per_token", &tails[0]).is_none()
        && named("input_cost_per_token", tier).is_none()
    {
        return None;
    }
    let input = price("input_cost_per_token")?;
    let output = price("output_cost_per_token")?;

    Some(Rates {
        input_usd_per_mtok: input,
        cached_input_usd_per_mtok: price("cache_read_input_token_cost").unwrap_or(input),
        cache_write_usd_per_mtok: price("cache_creation_input_token_cost").unwrap_or(input),
        output_usd_per_mtok: output,
    })
}

fn per_million(per_token: f64) -> f64 {
    (per_token * TOKENS_PER_MILLION * RATE_PRECISION).round() / RATE_PRECISION
}

/// A range bound is written as `256000.0`, and std converts a float to an integer only
/// through a lossy cast, so the integral text is parsed instead.
fn tokens(bound: &Value) -> Option<i64> {
    bound.as_i64().or_else(|| {
        bound
            .as_f64()
            .and_then(|value| format!("{value:.0}").parse().ok())
    })
}
