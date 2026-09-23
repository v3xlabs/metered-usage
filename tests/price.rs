use std::sync::Arc;

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
        ModelPrice::fill(&self.state.database).await?;
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
            "#
        ),
        |_| Some("a-secret-that-is-long".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let token = Token::try_from(TOKEN.to_owned())?;
    let state = Arc::new(AppState::new(database, token, config)?);
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
