use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use jiff::Timestamp;
use metered_usage::account::{AuthKind, NewAccount};
use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::id::Id;
use metered_usage::price::ModelPrice;
use metered_usage::source::Source;
use metered_usage::usage::NewUsageEvent;
use metered_usage::usage::ingest::{Incoming, ingest};
use poem::handler;
use poem::http::{StatusCode, header};
use poem::listener::{Acceptor, Listener, TcpListener};
use poem::test::TestClient;
use poem::web::{Data, Json};
use poem::{Endpoint, EndpointExt, Route, Server};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

const TOKEN: &str = "correct horse battery staple";
const TOLERANCE: f64 = 1e-12;
const SONNET: &str = "claude-sonnet-4-5";

type Failure = Box<dyn std::error::Error>;

#[derive(Clone)]
struct Maps {
    public: Arc<Mutex<Value>>,
    /// `None` answers 503, as a LiteLLM proxy that is down would.
    live: Arc<Mutex<Option<Value>>>,
}

struct Harness<E> {
    state: Arc<AppState>,
    client: TestClient<E>,
    maps: Maps,
}

#[derive(Debug, Deserialize)]
struct SyncOutput {
    inserted: i64,
    unchanged: i64,
    failed_sources: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Listing {
    prices: Vec<Price>,
}

#[derive(Debug, Deserialize)]
struct Price {
    #[serde(rename = "price_id")]
    id: String,
    provider: Option<String>,
    min_input_tokens: i64,
    origin: String,
    effective_from: String,
    input_usd_per_mtok: f64,
}

#[derive(Debug, Deserialize)]
struct AccountList {
    accounts: Vec<ListedAccount>,
}

#[derive(Debug, Deserialize)]
struct ListedAccount {
    account_id: String,
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Leverage {
    plans: Vec<PlanLeverage>,
}

#[derive(Debug, Deserialize)]
struct PlanLeverage {
    periods: Vec<LeveragePeriod>,
}

#[derive(Debug, Deserialize)]
struct LeveragePeriod {
    period_start: String,
    period_end: String,
    complete: bool,
    requests: i64,
    total_tokens: i64,
    list_cost_usd: f64,
    leverage: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct Cost {
    price_id: Option<i64>,
    list_cost_usd: Option<f64>,
    billed_cost_usd: Option<f64>,
}

impl<E: Endpoint> Harness<E> {
    async fn store(&self, source: &str, events: Vec<NewUsageEvent>) -> Result<(), Failure> {
        let source = Source::by_key(&self.state.database, source)
            .await?
            .ok_or("a configured source")?;
        let report = ingest(
            &self.state,
            source.id,
            events
                .into_iter()
                .map(|event| Incoming::Record(Box::new(event)))
                .collect(),
        )
        .await?;
        assert_eq!(report.rejected, 0);
        Ok(())
    }

    async fn sync(&self) -> Result<SyncOutput, Failure> {
        let synced = self.client.post("/api/prices/sync").send().await;
        synced.assert_status_is_ok();
        Ok(synced.json().await.value().deserialize())
    }

    async fn fill(&self) -> Result<(), Failure> {
        ModelPrice::fill(&self.state).await?;
        Ok(())
    }

    async fn cost(&self, upstream_id: &str) -> Result<Cost, Failure> {
        Ok(sqlx::query_as::<_, Cost>(
            "SELECT price_id, list_cost_usd, billed_cost_usd FROM usage_event \
             WHERE upstream_id = ?",
        )
        .bind(upstream_id)
        .fetch_one(&self.state.database.pool)
        .await?)
    }

    async fn prices(&self, query: &str) -> Result<Vec<Price>, Failure> {
        let listed = self.client.get(format!("/api/prices?{query}")).send().await;
        listed.assert_status_is_ok();
        Ok(listed.json().await.value().deserialize::<Listing>().prices)
    }

    async fn manual(&self, price: &Value) -> Result<Price, Failure> {
        let created = self
            .client
            .post("/api/prices")
            .body_json(price)
            .send()
            .await;
        created.assert_status_is_ok();
        Ok(created.json().await.value().deserialize())
    }
}

#[handler]
async fn public_map(Data(maps): Data<&Maps>) -> Json<Value> {
    Json(maps.public.lock().await.clone())
}

#[handler]
async fn live_map(Data(maps): Data<&Maps>) -> poem::Result<Json<Value>> {
    maps.live
        .lock()
        .await
        .clone()
        .map(Json)
        .ok_or_else(|| poem::Error::from_status(StatusCode::SERVICE_UNAVAILABLE))
}

fn public() -> Value {
    json!({
        "sample_spec": {
            "input_cost_per_token": 0.0,
            "output_cost_per_token": 0.0,
            "litellm_provider": "one of https://docs.litellm.ai/docs/providers",
        },
        SONNET: {
            "litellm_provider": "anthropic",
            "input_cost_per_token": 3e-06,
            "output_cost_per_token": 1.5e-05,
            "cache_read_input_token_cost": 3e-07,
            "cache_creation_input_token_cost": 3.75e-06,
            "cache_creation_input_token_cost_above_1hr": 6e-06,
            "input_cost_per_token_above_200k_tokens": 6e-06,
            "output_cost_per_token_above_200k_tokens": 2.25e-05,
            "cache_read_input_token_cost_above_200k_tokens": 6e-07,
            "cache_creation_input_token_cost_above_200k_tokens": 7.5e-06,
        },
        "gpt-5": {
            "litellm_provider": "openai",
            "input_cost_per_token": 1.25e-06,
            "output_cost_per_token": 1e-05,
            "input_cost_per_token_priority": 2.5e-06,
            "output_cost_per_token_priority": 2e-05,
        },
        "1024-x-1024/stable-diffusion": {
            "litellm_provider": "bedrock",
            "output_cost_per_image": 0.04,
        },
    })
}

fn live() -> Value {
    json!({
        "grok-3-mini": {
            "litellm_provider": "xai",
            "input_cost_per_token": 3e-07,
            "output_cost_per_token": 5e-07,
        },
    })
}

async fn harness() -> Result<Harness<impl Endpoint>, Failure> {
    let maps = Maps {
        public: Arc::new(Mutex::new(public())),
        live: Arc::new(Mutex::new(Some(live()))),
    };
    let acceptor = TcpListener::bind("127.0.0.1:0").into_acceptor().await?;
    let address = acceptor
        .local_addr()
        .first()
        .and_then(|address| address.as_socket_addr().copied())
        .ok_or("a socket address")?;
    let upstream = Route::new()
        .at("/map.json", poem::get(public_map))
        .at("/public/litellm_model_cost_map", poem::get(live_map))
        .data(maps.clone());
    tokio::spawn(Server::new_with_acceptor(acceptor).run(upstream));

    let config = Config::parse(
        &format!(
            r#"
            [[source]]
            key = "proxy"
            kind = "cliproxy"
            base_url = "http://127.0.0.1:1"
            key_env = "PROXY_KEY"

            [[source]]
            key = "lite"
            kind = "litellm"
            base_url = "http://{address}"
            key_env = "LITE_KEY"

            [pricing]
            litellm_source = "lite"
            public_map_url = "http://{address}/map.json"

            [pricing.priced_as]
            "kimi-k3-256k" = "moonshot/kimi-k3"
            "#
        ),
        |_| Some("a-secret-that-is-long".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let token = Token::try_from(TOKEN.to_owned())?;
    let state = Arc::new(AppState::new(database, token.into(), config)?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));

    Ok(Harness {
        state,
        client,
        maps,
    })
}

fn at(instant: &str) -> Result<Timestamp, Failure> {
    Ok(instant.parse()?)
}

/// A Claude subscription request through CLIProxy with no cache use.
fn request(upstream_id: &str, occurred_at: Timestamp, input: i64, output: i64) -> NewUsageEvent {
    NewUsageEvent {
        upstream_id: upstream_id.to_owned(),
        occurred_at,
        provider: "claude".to_owned(),
        model: SONNET.to_owned(),
        model_alias: None,
        endpoint: "POST /v1/messages".to_owned(),
        account: NewAccount::new(
            "auth-personal".to_owned(),
            AuthKind::OAuth,
            Some("person@example.com"),
        ),
        caller: None,
        harness: None,
        user_agent: None,
        session_id: None,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        unclassified_tokens: 0,
        total_tokens: input + output,
        token_quality: None,
        latency_ms: None,
        ttft_ms: None,
        streamed: false,
        status_code: 200,
        failed: false,
        error_message: None,
        service_tier: None,
        reasoning_effort: None,
        billed_cost_usd: None,
    }
}

fn raw(price: &Price) -> Result<i64, Failure> {
    Ok(price.id.parse::<Id<ModelPrice>>()?.raw())
}

fn close(actual: Option<f64>, expected: f64) {
    assert!(
        actual.is_some_and(|actual| (actual - expected).abs() < TOLERANCE),
        "{actual:?} is not {expected}"
    );
}

#[tokio::test]
async fn a_sync_records_each_price_once_and_survives_a_failed_source() -> Result<(), Failure> {
    let harness = harness().await?;

    let first = harness.sync().await?;
    assert_eq!(first.failed_sources, Vec::<String>::new());
    assert_eq!(
        first.inserted, 5,
        "two sonnet thresholds, two gpt-5 tiers, one live"
    );

    let second = harness.sync().await?;
    assert_eq!((second.inserted, second.unchanged), (0, 5));

    *harness.maps.live.lock().await = None;
    let degraded = harness.sync().await?;
    assert_eq!(degraded.failed_sources, vec!["litellm_live".to_owned()]);
    assert_eq!(degraded.unchanged, 4, "the public map still lands");

    let gpt = harness.prices("model=gpt-5").await?;
    assert_eq!(gpt.len(), 2);
    assert!(gpt.iter().all(|price| price.origin == "litellm_public"));

    Ok(())
}

#[tokio::test]
async fn a_changed_price_is_new_history_and_an_old_event_keeps_its_price() -> Result<(), Failure> {
    let harness = harness().await?;
    harness
        .store(
            "proxy",
            vec![request("before", at("2025-06-01T00:00:00Z")?, 1000, 0)],
        )
        .await?;

    harness.sync().await?;
    let original = harness.prices(&format!("model={SONNET}")).await?;
    let base = original
        .iter()
        .find(|price| price.min_input_tokens == 0)
        .ok_or("a base price")?;
    assert_eq!(
        base.effective_from, "1970-01-01T00:00:00Z",
        "the first price covers events older than the first sync"
    );
    assert_eq!(harness.cost("before").await?.price_id, Some(raw(base)?));

    harness.maps.public.lock().await[SONNET]["input_cost_per_token"] = json!(4e-06);
    assert_eq!(harness.sync().await?.inserted, 1);
    let history = harness
        .prices(&format!("model={SONNET}&history=true"))
        .await?;
    assert_eq!(history.len(), 3);
    let latest = harness.prices(&format!("model={SONNET}")).await?;
    let changed = latest
        .iter()
        .find(|price| price.min_input_tokens == 0)
        .ok_or("a base price")?;
    close(Some(changed.input_usd_per_mtok), 4.0);

    harness
        .store("proxy", vec![request("after", Timestamp::now(), 1000, 0)])
        .await?;
    harness.fill().await?;
    assert_eq!(harness.cost("before").await?.price_id, Some(raw(base)?));
    assert_eq!(harness.cost("after").await?.price_id, Some(raw(changed)?));

    let repriced = harness
        .client
        .post("/api/prices/reprice")
        .body_json(&json!({}))
        .send()
        .await;
    repriced.assert_status_is_ok();
    assert_eq!(
        harness.cost("before").await?.price_id,
        Some(raw(base)?),
        "a reprice uses the price in force when the request happened"
    );

    Ok(())
}

#[tokio::test]
async fn a_manual_price_outranks_a_synced_one() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    let manual = harness
        .manual(&json!({
            "model": SONNET,
            "effective_from": "2025-01-01T00:00:00Z",
            "input_usd_per_mtok": 1.0,
            "cached_input_usd_per_mtok": 0.1,
            "cache_write_usd_per_mtok": 1.25,
            "output_usd_per_mtok": 5.0,
        }))
        .await?;
    assert_eq!(manual.origin, "manual");

    harness
        .store(
            "proxy",
            vec![request("manual", at("2025-06-01T00:00:00Z")?, 1000, 1000)],
        )
        .await?;
    harness.fill().await?;

    let cost = harness.cost("manual").await?;
    assert_eq!(cost.price_id, Some(raw(&manual)?));
    close(cost.list_cost_usd, 0.006);

    Ok(())
}

#[tokio::test]
async fn a_gateway_provider_name_does_not_hide_a_synced_price() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    let scoped = harness
        .manual(&json!({
            "provider": "anthropic",
            "model": SONNET,
            "effective_from": "2025-01-01T00:00:00Z",
            "input_usd_per_mtok": 1.0,
            "cached_input_usd_per_mtok": 1.0,
            "cache_write_usd_per_mtok": 1.0,
            "output_usd_per_mtok": 1.0,
        }))
        .await?;
    assert_eq!(scoped.provider.as_deref(), Some("anthropic"));

    harness
        .store(
            "proxy",
            vec![request("claude", at("2025-06-01T00:00:00Z")?, 1000, 0)],
        )
        .await?;
    harness.fill().await?;

    let synced = harness
        .prices(&format!("model={SONNET}&origin=litellm_public"))
        .await?;
    let base = synced
        .iter()
        .find(|price| price.min_input_tokens == 0)
        .ok_or("a synced base price")?;
    assert_eq!(base.provider.as_deref(), Some("anthropic"));
    assert_eq!(
        harness.cost("claude").await?.price_id,
        Some(raw(base)?),
        "a synced row prices a claude event; the manual row scoped to anthropic does not"
    );

    Ok(())
}

#[tokio::test]
async fn a_key_litellm_names_only_under_its_provider_prices_the_bare_model() -> Result<(), Failure>
{
    let harness = harness().await?;
    {
        let mut map = harness.maps.public.lock().await;
        map["meta/muse-wonder-1.0"] = json!({
            "litellm_provider": "meta",
            "input_cost_per_token": 2e-06,
            "output_cost_per_token": 8e-06,
        });
        map["muse-spark-1.3"] = json!({
            "litellm_provider": "meta",
            "input_cost_per_token": 1e-06,
            "output_cost_per_token": 1e-06,
        });
        map["meta/muse-spark-1.3"] = json!({
            "litellm_provider": "meta",
            "input_cost_per_token": 9e-06,
            "output_cost_per_token": 9e-06,
        });
    }
    harness.sync().await?;
    let muse = |upstream_id: &str, provider: &str, model: &str| {
        let mut event = request(upstream_id, at("2025-06-01T00:00:00Z")?, 1000, 1000);
        provider.clone_into(&mut event.provider);
        model.clone_into(&mut event.model);
        Ok::<_, Failure>(event)
    };
    harness
        .store(
            "proxy",
            vec![
                muse("meta", "meta", "muse-wonder-1.0")?,
                muse("other", "kimi", "muse-wonder-1.0")?,
                muse("bare", "meta", "muse-spark-1.3")?,
            ],
        )
        .await?;
    harness.fill().await?;

    close(harness.cost("meta").await?.list_cost_usd, 0.01);
    close(harness.cost("bare").await?.list_cost_usd, 0.002);
    assert_eq!(
        harness.cost("other").await?.price_id,
        None,
        "another provider's request is not priced from the meta key"
    );

    Ok(())
}

#[tokio::test]
async fn a_model_the_map_knows_by_another_name_is_priced_as_configured() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.maps.public.lock().await["moonshot/kimi-k3"] = json!({
        "litellm_provider": "moonshot",
        "input_cost_per_token": 1e-06,
        "output_cost_per_token": 4e-06,
    });
    harness.sync().await?;
    let kimi = |upstream_id: &str, model: &str| {
        let mut event = request(upstream_id, at("2025-06-01T00:00:00Z")?, 1000, 1000);
        model.clone_into(&mut event.model);
        Ok::<_, Failure>(event)
    };
    harness
        .store(
            "proxy",
            vec![
                kimi("mapped", "kimi-k3-256k")?,
                kimi("unmapped", "kimi-k3-128k")?,
            ],
        )
        .await?;
    harness.fill().await?;

    close(harness.cost("mapped").await?.list_cost_usd, 0.005);
    assert_eq!(harness.cost("unmapped").await?.price_id, None);

    Ok(())
}

#[tokio::test]
async fn the_long_context_price_starts_above_its_threshold() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    harness
        .store(
            "proxy",
            vec![
                request("at", at("2025-06-01T00:00:00Z")?, 200_000, 1000),
                request("above", at("2025-06-01T00:00:01Z")?, 200_001, 1000),
            ],
        )
        .await?;
    harness.fill().await?;

    close(harness.cost("at").await?.list_cost_usd, 0.615);
    close(harness.cost("above").await?.list_cost_usd, 1.222_506);

    Ok(())
}

#[tokio::test]
async fn a_threshold_bound_is_inclusive_only_where_litellm_says() -> Result<(), Failure> {
    let harness = harness().await?;
    {
        let mut map = harness.maps.public.lock().await;
        map["grok-4"] = json!({
            "litellm_provider": "xai",
            "input_cost_per_token": 3e-06,
            "output_cost_per_token": 1.5e-05,
            "input_cost_per_token_above_200k_tokens": 6e-06,
            "output_cost_per_token_above_200k_tokens": 3e-05,
        });
        map["qwen-plus"] = json!({
            "litellm_provider": "dashscope",
            "tiered_pricing": [
                { "input_cost_per_token": 4e-07, "output_cost_per_token": 1.2e-06, "range": [0, 256_000.0] },
                { "input_cost_per_token": 1.2e-06, "output_cost_per_token": 3.6e-06, "range": [256_000.0, 1_000_000.0] },
            ],
        });
    }
    harness.sync().await?;
    let priced = |upstream_id: &str, model: &str, input: i64| {
        let mut event = request(upstream_id, at("2025-06-01T00:00:00Z")?, input, 0);
        event.model = model.to_owned();
        Ok::<_, Failure>(event)
    };
    harness
        .store(
            "proxy",
            vec![
                priced("grok-at", "grok-4", 200_000)?,
                priced("qwen-at", "qwen-plus", 256_000)?,
                priced("qwen-above", "qwen-plus", 256_001)?,
            ],
        )
        .await?;
    harness.fill().await?;

    // xAI bills the long-context rate from the threshold itself: 200 000 at 6.
    close(harness.cost("grok-at").await?.list_cost_usd, 1.2);
    // A tier range holds its end and not its start: 256 000 at 0.40, 256 001 at 1.20.
    close(harness.cost("qwen-at").await?.list_cost_usd, 0.1024);
    close(harness.cost("qwen-above").await?.list_cost_usd, 0.307_201_2);

    Ok(())
}

#[tokio::test]
async fn a_rate_a_tier_or_threshold_leaves_out_falls_back_as_litellm_does() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.maps.public.lock().await["gpt-5.4"] = json!({
        "litellm_provider": "openai",
        "input_cost_per_token": 2.5e-06,
        "output_cost_per_token": 1.5e-05,
        "cache_read_input_token_cost": 2.5e-07,
        "input_cost_per_token_flex": 1.25e-06,
        "output_cost_per_token_flex": 7.5e-06,
        "input_cost_per_token_above_272k_tokens": 5e-06,
        "output_cost_per_token_above_272k_tokens": 2.25e-05,
    });
    harness.sync().await?;
    let cached = |upstream_id: &str, input: i64, cache_read: i64, tier: Option<&str>| {
        let mut event = request(upstream_id, at("2025-06-01T00:00:00Z")?, input, 1000);
        event.model = "gpt-5.4".to_owned();
        event.cache_read_tokens = cache_read;
        event.service_tier = tier.map(str::to_owned);
        Ok::<_, Failure>(event)
    };
    harness
        .store(
            "proxy",
            vec![
                cached("flex", 10_000, 8000, Some("flex"))?,
                cached("long", 300_000, 290_000, None)?,
                cached("flex-long", 300_000, 290_000, Some("flex"))?,
            ],
        )
        .await?;
    harness.fill().await?;

    // 2000 uncached at the flex 1.25, 8000 read at the base 0.25, 1000 out at the flex 7.5.
    close(harness.cost("flex").await?.list_cost_usd, 0.012);
    // 10 000 uncached at 5, 290 000 read at the base 0.25, 1000 out at 22.5.
    close(harness.cost("long").await?.list_cost_usd, 0.145);
    // Flex names no long-context rate, so the long-context rates apply.
    close(harness.cost("flex-long").await?.list_cost_usd, 0.145);

    Ok(())
}

#[tokio::test]
async fn cache_tokens_are_priced_apart_from_the_input_they_are_part_of() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    let mut cached = request("cached", at("2025-06-01T00:00:00Z")?, 10_000, 2000);
    cached.cache_read_tokens = 6000;
    cached.cache_write_tokens = 1000;
    cached.reasoning_tokens = 500;
    let mut overlapping = request("overlapping", at("2025-06-01T00:00:01Z")?, 100, 0);
    overlapping.cache_read_tokens = 150;
    harness.store("proxy", vec![cached, overlapping]).await?;
    harness.fill().await?;

    // 3000 uncached at 3, 6000 read at 0.30, 1000 written at 3.75, 2000 out at 15.
    close(harness.cost("cached").await?.list_cost_usd, 0.044_55);
    // More cache read than input leaves no uncached input to charge below zero.
    close(harness.cost("overlapping").await?.list_cost_usd, 0.000_045);

    Ok(())
}

#[tokio::test]
async fn a_subscription_bills_nothing_and_an_api_key_bills_list() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    let mut keyed = request("keyed", at("2025-06-01T00:00:00Z")?, 1000, 1000);
    keyed.account = NewAccount::new(
        "auth-key".to_owned(),
        AuthKind::ApiKey,
        Some("sk-ant-0123456789abcd"),
    );
    harness
        .store(
            "proxy",
            vec![
                request("oauth", at("2025-06-01T00:00:00Z")?, 1000, 1000),
                keyed,
            ],
        )
        .await?;
    let mut billed = request("litellm", at("2025-06-01T00:00:00Z")?, 1000, 1000);
    billed.billed_cost_usd = Some(0.123);
    billed.account = NewAccount::new("deployment-1".to_owned(), AuthKind::ApiKey, None);
    harness.store("lite", vec![billed]).await?;
    harness.fill().await?;

    let oauth = harness.cost("oauth").await?;
    close(oauth.list_cost_usd, 0.018);
    close(oauth.billed_cost_usd, 0.0);
    let keyed = harness.cost("keyed").await?;
    close(keyed.billed_cost_usd, 0.018);
    let litellm = harness.cost("litellm").await?;
    close(litellm.list_cost_usd, 0.018);
    close(litellm.billed_cost_usd, 0.123);

    harness
        .manual(&json!({
            "model": SONNET,
            "effective_from": "2025-01-01T00:00:00Z",
            "input_usd_per_mtok": 1.0,
            "cached_input_usd_per_mtok": 1.0,
            "cache_write_usd_per_mtok": 1.0,
            "output_usd_per_mtok": 1.0,
        }))
        .await?;
    let repriced = harness
        .client
        .post("/api/prices/reprice")
        .body_json(&json!({ "from": "2025-06-01T00:00:00Z", "to": "2025-06-01T00:00:00Z" }))
        .send()
        .await;
    repriced.assert_status_is_ok();
    repriced.assert_json(json!({ "repriced": 3 })).await;

    let litellm = harness.cost("litellm").await?;
    close(litellm.list_cost_usd, 0.002);
    close(litellm.billed_cost_usd, 0.123);
    close(harness.cost("keyed").await?.billed_cost_usd, 0.002);
    close(harness.cost("oauth").await?.billed_cost_usd, 0.0);

    Ok(())
}

#[tokio::test]
async fn leverage_is_counted_per_billing_period() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    let mut other = request("other", at("2025-02-15T00:00:00Z")?, 100_000, 0);
    other.account = NewAccount::new(
        "auth-work".to_owned(),
        AuthKind::OAuth,
        Some("work@example.com"),
    );
    harness
        .store(
            "proxy",
            vec![
                request("before", at("2025-01-30T00:00:00Z")?, 100_000, 0),
                request("february", at("2025-02-15T00:00:00Z")?, 100_000, 0),
                request("clamped", at("2025-02-28T12:00:00Z")?, 100_000, 0),
                request("march", at("2025-03-10T00:00:00Z")?, 100_000, 0),
                other,
            ],
        )
        .await?;
    harness.fill().await?;

    let listed = harness.client.get("/api/accounts").send().await;
    listed.assert_status_is_ok();
    let personal = listed
        .json()
        .await
        .value()
        .deserialize::<AccountList>()
        .accounts
        .into_iter()
        .find(|account| account.label.as_deref() == Some("person@example.com"))
        .ok_or("the personal account")?;
    let created = harness
        .client
        .post("/api/plans")
        .body_json(&json!({
            "account_id": personal.account_id,
            "name": "Max",
            "monthly_usd": 0.2,
            "period_start": "2025-01-31T00:00:00Z",
            "period_end": "2025-03-31T00:00:00Z",
        }))
        .send()
        .await;
    created.assert_status_is_ok();

    let response = harness.client.get("/api/leverage").send().await;
    response.assert_status_is_ok();
    let leverage = response.json().await.value().deserialize::<Leverage>();
    let [plan] = leverage.plans.as_slice() else {
        return Err(format!("one plan, found {}", leverage.plans.len()).into());
    };
    let [march, february] = plan.periods.as_slice() else {
        return Err(format!("two periods, found {:?}", plan.periods).into());
    };

    assert_eq!(
        (march.period_start.as_str(), march.period_end.as_str()),
        ("2025-02-28T00:00:00Z", "2025-03-31T00:00:00Z"),
        "a period anchored on the 31st starts on the last day of February"
    );
    assert_eq!(march.requests, 2);
    assert_eq!(march.total_tokens, 200_000);
    close(Some(march.list_cost_usd), 0.6);
    close(march.leverage, 3.0);
    assert!(march.complete);

    assert_eq!(
        (february.period_start.as_str(), february.period_end.as_str()),
        ("2025-01-31T00:00:00Z", "2025-02-28T00:00:00Z")
    );
    assert_eq!(february.requests, 1);
    close(Some(february.list_cost_usd), 0.3);
    close(february.leverage, 1.5);

    Ok(())
}

#[tokio::test]
async fn an_open_stream_learns_the_cost_of_an_event_priced_after_it_was_sent() -> Result<(), Failure>
{
    let harness = harness().await?;
    let stream = harness.client.get("/api/usage/stream").send().await;
    stream.assert_status_is_ok();
    let mut body = stream.0.into_body().into_bytes_stream();

    harness
        .store(
            "proxy",
            vec![request("late", at("2025-06-01T00:00:00Z")?, 1000, 1000)],
        )
        .await?;
    let manual = harness
        .manual(&json!({
            "model": SONNET,
            "effective_from": "2025-01-01T00:00:00Z",
            "input_usd_per_mtok": 1.0,
            "cached_input_usd_per_mtok": 0.1,
            "cache_write_usd_per_mtok": 1.25,
            "output_usd_per_mtok": 5.0,
        }))
        .await?;
    harness.fill().await?;

    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while text.matches("\n\n").count() < 2 {
            let chunk = body
                .next()
                .await
                .ok_or("the stream ended")?
                .map_err(|error| error.to_string())?;
            text.push_str(std::str::from_utf8(&chunk).map_err(|error| error.to_string())?);
        }
        Ok::<_, String>(())
    })
    .await??;

    let messages = text
        .split("\n\n")
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>();
    let field = |message: &str, name: &str| {
        message
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::to_owned)
    };
    let [usage, priced] = messages.as_slice() else {
        return Err(format!("a usage and a priced message, found {text:?}").into());
    };
    assert_eq!(field(usage, "event: ").as_deref(), Some("usage"));
    let usage = serde_json::from_str::<Value>(&field(usage, "data: ").ok_or("usage data")?)?;
    assert_eq!(usage["list_cost_usd"], Value::Null, "sent before any price");

    assert_eq!(field(priced, "event: ").as_deref(), Some("priced"));
    assert_eq!(
        field(priced, "id: "),
        None,
        "Last-Event-ID keeps naming usage events"
    );
    let priced = serde_json::from_str::<Value>(&field(priced, "data: ").ok_or("priced data")?)?;
    assert_eq!(priced["event_id"], usage["event_id"]);
    assert_eq!(priced["price_id"], json!(manual.id));
    close(priced["list_cost_usd"].as_f64(), 0.006);
    close(priced["billed_cost_usd"].as_f64(), 0.0);

    Ok(())
}

async fn seed_litellm_effort_slice(harness: &Harness<impl Endpoint>) -> Result<(), Failure> {
    let slice: Value = serde_json::from_str(include_str!("fixtures/litellm_effort_slice.json"))?;
    let mut map = harness.maps.public.lock().await;
    if let Some(entries) = slice.as_object() {
        for (key, value) in entries {
            map[key] = value.clone();
        }
    }
    for (key, provider, input, output) in [
        ("gemini/gemini-foo-high", "gemini", 0.0, 0.0),
        ("gemini/gemini-foo", "gemini", 2e-06, 12e-06),
        ("anthropic/claude-x", "anthropic", 9e-06, 9e-06),
        ("foo", "openai", 1e-06, 2e-06),
        ("foo-high", "openai", 9e-06, 9e-06),
        ("bar", "openai", 3e-06, 6e-06),
        ("llama", "openai", 1e-06, 1e-06),
        ("groq/llama-high", "groq", 7e-06, 7e-06),
        ("cerebras/llama-high", "cerebras", 8e-06, 8e-06),
        ("qwen3", "openai", 1e-06, 1e-06),
        ("alias-pub", "moonshot", 1e-06, 1e-06),
    ] {
        map[key] = json!({
            "litellm_provider": provider,
            "input_cost_per_token": input,
            "output_cost_per_token": output,
        });
    }
    drop(map);
    harness.sync().await?;
    harness
        .manual(&json!({
            "model": "moonshot/kimi-k3",
            "input_usd_per_mtok": 5.0,
            "cached_input_usd_per_mtok": 5.0,
            "cache_write_usd_per_mtok": 5.0,
            "output_usd_per_mtok": 5.0,
            "effective_from": "2020-01-01T00:00:00Z",
        }))
        .await?;
    Ok(())
}

async fn priced_model_key(
    harness: &Harness<impl Endpoint>,
    upstream_id: &str,
) -> Result<Option<String>, Failure> {
    Ok(sqlx::query_scalar(
        "SELECT mp.model FROM usage_event ue \
         LEFT JOIN model_price mp ON mp.price_id = ue.price_id \
         WHERE ue.upstream_id = ?",
    )
    .bind(upstream_id)
    .fetch_one(&harness.state.database.pool)
    .await?)
}

#[tokio::test]
async fn effort_suffix_litellm_slice() -> Result<(), Failure> {
    let harness = harness().await?;
    seed_litellm_effort_slice(&harness).await?;

    let event = |upstream_id: &str, provider: &str, model: &str| {
        let mut row = request(upstream_id, at("2025-06-01T00:00:00Z")?, 1000, 1000);
        provider.clone_into(&mut row.provider);
        model.clone_into(&mut row.model);
        Ok::<_, Failure>(row)
    };
    let mut api_cut = event("bar-cut", "openai", "bar-high")?;
    api_cut.account = NewAccount::new("auth-api".to_owned(), AuthKind::ApiKey, Some("sk-1"));
    let mut manual_over_alias = event("ranked", "moonshot", "kimi-k3-256k")?;
    manual_over_alias.model_alias = Some("alias-pub".to_owned());

    harness
        .store(
            "proxy",
            vec![
                event("neon-xhigh", "antigravity", "gemini-4-neon-xhigh")?,
                event("neon-cli", "gemini-cli", "gemini-4-neon-high")?,
                event("argon-high", "antigravity", "gemini-4-argon-high")?,
                event("carbon-high", "antigravity", "gemini-4-carbon-high")?,
                event("hydrogen-xhigh", "antigravity", "gemini-4-hydrogen-xhigh")?,
                event("priced-as-base", "moonshot", "kimi-k3-256k-high")?,
                event("grok-unique", "xai", "grok-4-high")?,
                event("mistral", "mistral", "mistral-medium")?,
                event("foo-priced", "openai", "foo-high")?,
                api_cut,
                manual_over_alias,
                event("free-exact", "antigravity", "gemini-foo-high")?,
                event("unsuffixed-claude", "claude", "claude-x")?,
                event("qwen", "qwen", "qwen-max")?,
                event("two-provider-exact", "antigravity", "llama-high")?,
                event("two-slash-exact", "antigravity", "qwen3-max")?,
                event("gpt-oss-ambig", "antigravity", "gpt-oss-120b-medium")?,
                event("prefixed-event", "antigravity", "azure/gemini-4-neon-high")?,
                event("something", "antigravity", "something-high")?,
                event("highspeed", "moonshot", "kimi-k3.1-pro-highspeed")?,
            ],
        )
        .await?;
    harness.fill().await?;

    for (id, expected_key, expected_list, expect_billed_none) in [
        ("neon-xhigh", "gemini/gemini-4-neon", 0.018, true),
        ("neon-cli", "gemini/gemini-4-neon", 0.018, true),
        ("argon-high", "gemini/gemini-4-argon", 0.012, true),
        ("carbon-high", "gemini/gemini-4-carbon", 0.012, true),
        ("hydrogen-xhigh", "gemini/gemini-4-hydrogen", 0.009, true),
        ("priced-as-base", "moonshot/kimi-k3", 0.01, true),
        ("grok-unique", "azure_ai/grok-4", 0.018, true),
        ("mistral", "mistral/mistral-medium", 0.009, false),
        ("foo-priced", "foo-high", 0.018, false),
        ("bar-cut", "bar", 0.009, true),
        ("ranked", "moonshot/kimi-k3", 0.01, false),
    ] {
        assert_eq!(
            priced_model_key(&harness, id).await?.as_deref(),
            Some(expected_key),
            "{id} chosen key"
        );
        let cost = harness.cost(id).await?;
        close(cost.list_cost_usd, expected_list);
        if expect_billed_none {
            assert_eq!(cost.billed_cost_usd, None, "{id} billed stays NULL");
        }
    }
    for unpriced in [
        "free-exact",
        "unsuffixed-claude",
        "qwen",
        "two-provider-exact",
        "two-slash-exact",
        "gpt-oss-ambig",
        "prefixed-event",
        "something",
        "highspeed",
    ] {
        assert_eq!(
            harness.cost(unpriced).await?.price_id,
            None,
            "{unpriced} stays unpriced"
        );
    }

    let stored_model: String =
        sqlx::query_scalar("SELECT model FROM usage_event WHERE upstream_id = 'neon-xhigh'")
            .fetch_one(&harness.state.database.pool)
            .await?;
    assert_eq!(stored_model, "gemini-4-neon-xhigh");

    Ok(())
}

#[tokio::test]
async fn effort_suffix_events() -> Result<(), Failure> {
    let harness = harness().await?;
    seed_litellm_effort_slice(&harness).await?;

    let event = |upstream_id: &str, occurred_at: &str, model: &str| {
        let mut row = request(upstream_id, at(occurred_at)?, 1000, 1000);
        "antigravity".clone_into(&mut row.provider);
        model.clone_into(&mut row.model);
        Ok::<_, Failure>(row)
    };
    let mut neon_with_stored_effort = event(
        "neon-stored",
        "2025-06-01T11:30:00Z",
        "gemini-4-neon-medium",
    )?;
    neon_with_stored_effort.model_alias = Some("gemini-4-neon-medium".to_owned());
    neon_with_stored_effort.reasoning_effort = Some("high".to_owned());

    harness
        .store(
            "proxy",
            vec![
                event("neon-xhigh", "2025-06-01T10:00:00Z", "gemini-4-neon-xhigh")?,
                neon_with_stored_effort,
                event("safe", "2025-06-01T12:00:00Z", "something-high")?,
            ],
        )
        .await?;
    harness.fill().await?;

    let events_response = harness.client.get("/api/usage/events").send().await;
    events_response.assert_status_is_ok();
    let events_json: Value = events_response.json().await.value().deserialize();
    let events = events_json["events"].as_array().ok_or("events array")?;
    let xhigh = events
        .iter()
        .find(|row| row["upstream_id"] == "neon-xhigh")
        .ok_or("neon-xhigh event")?;
    assert_eq!(xhigh["model"], "gemini-4-neon");
    assert_eq!(xhigh["raw_model"], "gemini-4-neon-xhigh");
    assert_eq!(xhigh.get("model_alias"), None);
    assert_eq!(xhigh["reasoning_effort"], "xhigh");

    let stored = events
        .iter()
        .find(|row| row["upstream_id"] == "neon-stored")
        .ok_or("neon-stored event")?;
    assert_eq!(stored["model"], "gemini-4-neon");
    assert_eq!(stored["raw_model"], "gemini-4-neon-medium");
    assert_eq!(stored["model_alias"], "gemini-4-neon-medium");
    assert_eq!(stored["reasoning_effort"], "high");

    let safe = events
        .iter()
        .find(|row| row["upstream_id"] == "safe")
        .ok_or("safe event")?;
    assert_eq!(safe["model"], "something-high");
    assert_eq!(safe.get("raw_model"), None);
    assert_eq!(safe.get("reasoning_effort"), None);

    for (query, expected_len) in [
        ("/api/usage/events?model=gemini-4-neon", 2),
        ("/api/usage/events?model=gemini-4-neon-xhigh", 1),
    ] {
        let filtered = harness.client.get(query).send().await;
        filtered.assert_status_is_ok();
        let filtered_json: Value = filtered.json().await.value().deserialize();
        assert_eq!(
            filtered_json["events"].as_array().ok_or("events")?.len(),
            expected_len
        );
    }

    Ok(())
}

#[tokio::test]
async fn an_event_stored_for_a_priced_model_has_its_cost_before_any_fill() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    let mut unlisted = request("unlisted", Timestamp::now(), 1000, 1000);
    "unlisted-model".clone_into(&mut unlisted.model);
    harness
        .store(
            "proxy",
            vec![request("listed", Timestamp::now(), 1000, 1000), unlisted],
        )
        .await?;

    let listed = harness.cost("listed").await?;
    close(listed.list_cost_usd, 0.018);
    close(listed.billed_cost_usd, 0.0);
    assert_eq!(harness.cost("unlisted").await?.list_cost_usd, None);

    harness
        .manual(&json!({
            "model": "unlisted-model",
            "effective_from": "2025-01-01T00:00:00Z",
            "input_usd_per_mtok": 1.0,
            "cached_input_usd_per_mtok": 0.1,
            "cache_write_usd_per_mtok": 1.25,
            "output_usd_per_mtok": 5.0,
        }))
        .await?;
    harness.fill().await?;
    close(harness.cost("unlisted").await?.list_cost_usd, 0.006);

    Ok(())
}

#[tokio::test]
async fn a_batch_whose_pricing_fails_is_still_stored_and_left_to_the_fill() -> Result<(), Failure> {
    let harness = harness().await?;
    harness.sync().await?;
    sqlx::query("ALTER TABLE model_price RENAME TO model_price_away")
        .execute(&harness.state.database.pool)
        .await?;
    harness
        .store("proxy", vec![request("kept", Timestamp::now(), 1000, 1000)])
        .await?;
    assert_eq!(harness.cost("kept").await?.list_cost_usd, None);

    sqlx::query("ALTER TABLE model_price_away RENAME TO model_price")
        .execute(&harness.state.database.pool)
        .await?;
    harness.fill().await?;
    close(harness.cost("kept").await?.list_cost_usd, 0.018);

    Ok(())
}
