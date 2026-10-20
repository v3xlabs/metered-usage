use std::collections::BTreeSet;
use std::sync::Arc;

use jiff::Timestamp;
use metered_usage::account::{AuthKind, NewAccount};
use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::{Credentials, Token};
use metered_usage::price::ModelPrice;
use metered_usage::source::Source;
use metered_usage::usage::NewUsageEvent;
use metered_usage::usage::ingest::{Incoming, ingest};
use poem::Endpoint;
use poem::http::{StatusCode, header};
use poem::test::{TestClient, TestResponse};
use serde::Deserialize;
use serde_json::{Value, json};

const TOKEN: &str = "correct horse battery staple";
const CONSUMER_TOKEN: &str = "a service that follows usage";
const TOLERANCE: f64 = 1e-9;
const SONNET: &str = "claude-sonnet-4-5";
const CODEX: &str = "gpt-5-codex";
/// From the first instant of 03-02 to 05:00 on 03-03, so `after` at 05:00 is left out.
const WINDOW: &str = "from=2026-03-02T00:00:00Z&to=2026-03-03T05:00:00Z";
/// More models than the analytics series keeps by default before it merges into `other`.
const MODELS: [&str; 12] = [
    "m-00", "m-01", "m-02", "m-03", "m-04", "m-05", "m-06", "m-07", "m-08", "m-09", "m-10", "m-11",
];

type Failure = Box<dyn std::error::Error>;

struct Harness<E> {
    client: TestClient<E>,
    anonymous: TestClient<E>,
    state: Arc<AppState>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    total: Metrics,
    groups: Option<Vec<Group>>,
    buckets: Vec<Bucket>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bucket {
    start: String,
    total: Metrics,
    groups: Option<Vec<Group>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    key: String,
    metrics: Metrics,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metrics {
    requests: i64,
    input_tokens: i64,
    uncached_input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    unclassified_tokens: i64,
    total_tokens: i64,
    list_cost_usd: f64,
    billed_cost_usd: f64,
    unpriced_requests: i64,
    unbilled_requests: i64,
}

#[derive(Debug, Deserialize)]
struct AccountList {
    accounts: Vec<Account>,
}

#[derive(Debug, Deserialize)]
struct Account {
    account_id: String,
    label: Option<String>,
}

/// One request, as `request` below describes it before the defaults are filled in.
#[derive(Clone, Copy)]
struct Request {
    upstream_id: &'static str,
    at: &'static str,
    model: &'static str,
    provider: &'static str,
    account: (&'static str, AuthKind),
    input: i64,
    cache_read: i64,
    cache_write: i64,
    output: i64,
    reasoning: i64,
    billed: Option<f64>,
    failed: bool,
}

impl<E: Endpoint> Harness<E> {
    async fn summary(&self, query: &str) -> Summary {
        let response = self
            .client
            .get(format!("/api/usage/summary?{query}"))
            .send()
            .await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize()
    }

    async fn refused(&self, query: &str) -> TestResponse {
        let response = self
            .client
            .get(format!("/api/usage/summary?{query}"))
            .send()
            .await;
        assert_eq!(response.0.status(), StatusCode::BAD_REQUEST, "{query}");
        response
    }

    async fn account(&self, label: &str) -> Result<String, Failure> {
        let response = self.client.get("/api/accounts").send().await;
        response.assert_status_is_ok();
        response
            .json()
            .await
            .value()
            .deserialize::<AccountList>()
            .accounts
            .into_iter()
            .find(|account| {
                account
                    .label
                    .as_deref()
                    .is_some_and(|shown| shown.ends_with(label))
            })
            .map(|account| account.account_id)
            .ok_or_else(|| format!("no account {label}").into())
    }

    async fn ingest(&self, key: &str, requests: &[Request]) -> Result<(), Failure> {
        let source = Source::by_key(&self.state.database, key)
            .await?
            .ok_or("a configured source")?;
        let records = requests
            .iter()
            .map(|each| request(each).map(|event| Incoming::Record(Box::new(event))))
            .collect::<Result<Vec<_>, _>>()?;
        let report = ingest(&self.state, source.id, records).await?;
        assert_eq!(report.accepted, i64::try_from(requests.len())?);
        Ok(())
    }

    async fn price(&self, model: &str, rates: [f64; 4]) {
        let [input, cached_input, cache_write, output] = rates;
        let created = self
            .client
            .post("/api/prices")
            .body_json(&json!({
                "model": model,
                "effective_from": "2025-01-01T00:00:00Z",
                "input_usd_per_mtok": input,
                "cached_input_usd_per_mtok": cached_input,
                "cache_write_usd_per_mtok": cache_write,
                "output_usd_per_mtok": output,
            }))
            .send()
            .await;
        created.assert_status_is_ok();
    }
}

impl Metrics {
    fn plus(mut self, other: &Self) -> Self {
        self.requests += other.requests;
        self.input_tokens += other.input_tokens;
        self.uncached_input_tokens += other.uncached_input_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.unclassified_tokens += other.unclassified_tokens;
        self.total_tokens += other.total_tokens;
        self.list_cost_usd += other.list_cost_usd;
        self.billed_cost_usd += other.billed_cost_usd;
        self.unpriced_requests += other.unpriced_requests;
        self.unbilled_requests += other.unbilled_requests;
        self
    }

    fn assert_same(&self, other: &Self, context: &str) {
        let counts = |metrics: &Self| {
            [
                metrics.requests,
                metrics.input_tokens,
                metrics.uncached_input_tokens,
                metrics.cache_read_tokens,
                metrics.cache_write_tokens,
                metrics.output_tokens,
                metrics.reasoning_tokens,
                metrics.unclassified_tokens,
                metrics.total_tokens,
                metrics.unpriced_requests,
                metrics.unbilled_requests,
            ]
        };
        assert_eq!(counts(self), counts(other), "{context}");
        close(self.list_cost_usd, other.list_cost_usd);
        close(self.billed_cost_usd, other.billed_cost_usd);
    }
}

fn request(request: &Request) -> Result<NewUsageEvent, Failure> {
    let (account, auth_kind) = request.account;
    Ok(NewUsageEvent {
        upstream_id: request.upstream_id.to_owned(),
        occurred_at: request.at.parse::<Timestamp>()?,
        provider: request.provider.to_owned(),
        model: request.model.to_owned(),
        model_alias: None,
        endpoint: "POST /v1/messages".to_owned(),
        account: NewAccount::new(
            account.to_owned(),
            auth_kind,
            Some(&format!("sk-account-{account}")),
        ),
        caller: Some("sk-client-masked".to_owned()),
        harness: Some("harness-omp".to_owned()),
        user_agent: Some("an agent/1.0".to_owned()),
        session_id: Some("a session".to_owned()),
        input_tokens: request.input,
        output_tokens: request.output,
        reasoning_tokens: request.reasoning,
        cache_read_tokens: request.cache_read,
        cache_write_tokens: request.cache_write,
        unclassified_tokens: 0,
        total_tokens: request.input + request.output,
        token_quality: None,
        latency_ms: Some(1000),
        ttft_ms: None,
        streamed: false,
        status_code: if request.failed { 500 } else { 200 },
        failed: request.failed,
        error_message: request.failed.then(|| "upstream overloaded".to_owned()),
        service_tier: None,
        reasoning_effort: None,
        billed_cost_usd: request.billed,
    })
}

/// Sonnet is priced by hand at 3 in, 0.30 cached, 3.75 cache write and 15 out per million;
/// Codex has no price. `f` carries the charge its gateway reported.
///
/// | event  | at (UTC)              | source | account  | model  | in        | read    | write   | out     | reasoning | list  | billed |
/// | before | 03-01 23:59:59        | alpha  | a1 oauth | sonnet | 1 000     | 0       | 0       | 0       | 0         | -     | -      |
/// | a      | 03-02 10:00:00        | alpha  | a1 oauth | sonnet | 1 000 000 | 600 000 | 100 000 | 100 000 | 0         | 2.955 | 0      |
/// | b      | 03-02 10:59:59.999    | alpha  | a2 key   | sonnet | 200 000   | 0       | 200 000 | 0       | 0         | 0.75  | 0.75   |
/// | c      | 03-02 11:00:00        | alpha  | a2 key   | sonnet | 1 000     | 0       | 0       | 0       | 0         | 0.003 | 0.003  |
/// | d      | 03-02 23:59:59 fails  | alpha  | a3 oauth | codex  | 2 000 000 | 0       | 0       | 5 000   | 1 000     | none  | none   |
/// | e      | 03-03 00:00:00        | beta   | b1 key   | sonnet | 100 000   | 0       | 0       | 10 000  | 4 000     | 0.45  | 0.45   |
/// | f      | 03-03 04:30:00        | beta   | b2 key   | codex  | 50 000    | 0       | 0       | 1 000   | 0         | none  | 0.25   |
/// | after  | 03-03 05:00:00        | beta   | b1 key   | sonnet | 1 000     | 0       | 0       | 0       | 0         | -     | -      |
async fn harness() -> Result<Harness<impl Endpoint>, Failure> {
    let config = Config::parse(
        r#"
        [[source]]
        key = "alpha"
        kind = "cliproxy"
        base_url = "http://127.0.0.1:1"
        key_env = "ALPHA_KEY"

        [[source]]
        key = "beta"
        kind = "cliproxy"
        base_url = "http://127.0.0.1:2"
        key_env = "BETA_KEY"
        "#,
        |_| Some("a-secret-that-is-long".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let credentials = Credentials {
        token: Token::try_from(TOKEN.to_owned())?,
        read_token: None,
        consumer_token: Some(Token::try_from(CONSUMER_TOKEN.to_owned())?),
    };
    let state = Arc::new(AppState::new(database, credentials, config)?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let anonymous = TestClient::new(metered_usage::http::routes(&state));

    let harness = Harness {
        client,
        anonymous,
        state,
    };
    harness.price(SONNET, [3.0, 0.3, 3.75, 15.0]).await;
    for (key, requests) in requests() {
        harness.ingest(key, &requests).await?;
    }
    ModelPrice::fill(&harness.state).await?;

    Ok(harness)
}

fn base() -> Request {
    Request {
        upstream_id: "",
        at: "",
        model: SONNET,
        provider: "claude",
        account: ("a1", AuthKind::OAuth),
        input: 0,
        cache_read: 0,
        cache_write: 0,
        output: 0,
        reasoning: 0,
        billed: None,
        failed: false,
    }
}

/// The events of the table on [`harness`].
fn requests() -> [(&'static str, Vec<Request>); 2] {
    let base = base();
    let alpha = vec![
        Request {
            upstream_id: "before",
            at: "2026-03-01T23:59:59Z",
            input: 1000,
            ..base
        },
        Request {
            upstream_id: "a",
            at: "2026-03-02T10:00:00Z",
            input: 1_000_000,
            cache_read: 600_000,
            cache_write: 100_000,
            output: 100_000,
            ..base
        },
        Request {
            upstream_id: "b",
            at: "2026-03-02T10:59:59.999Z",
            account: ("a2", AuthKind::ApiKey),
            input: 200_000,
            cache_write: 200_000,
            ..base
        },
        Request {
            upstream_id: "c",
            at: "2026-03-02T11:00:00Z",
            account: ("a2", AuthKind::ApiKey),
            input: 1000,
            ..base
        },
        Request {
            upstream_id: "d",
            at: "2026-03-02T23:59:59Z",
            model: CODEX,
            provider: "codex",
            account: ("a3", AuthKind::OAuth),
            input: 2_000_000,
            output: 5000,
            reasoning: 1000,
            failed: true,
            ..base
        },
    ];
    let beta = vec![
        Request {
            upstream_id: "e",
            at: "2026-03-03T00:00:00Z",
            account: ("b1", AuthKind::ApiKey),
            input: 100_000,
            output: 10_000,
            reasoning: 4000,
            ..base
        },
        Request {
            upstream_id: "f",
            at: "2026-03-03T04:30:00Z",
            model: CODEX,
            provider: "codex",
            account: ("b2", AuthKind::ApiKey),
            input: 50_000,
            output: 1000,
            billed: Some(0.25),
            ..base
        },
        Request {
            upstream_id: "after",
            at: "2026-03-03T05:00:00Z",
            account: ("b1", AuthKind::ApiKey),
            input: 1000,
            ..base
        },
    ];
    [("alpha", alpha), ("beta", beta)]
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{actual} is not {expected}"
    );
}

/// The window's totals of `a` to `f`.
fn window_total() -> Metrics {
    Metrics {
        requests: 6,
        input_tokens: 3_351_000,
        uncached_input_tokens: 2_451_000,
        cache_read_tokens: 600_000,
        cache_write_tokens: 300_000,
        output_tokens: 116_000,
        reasoning_tokens: 5000,
        unclassified_tokens: 0,
        total_tokens: 3_467_000,
        list_cost_usd: 4.158,
        billed_cost_usd: 1.453,
        unpriced_requests: 2,
        unbilled_requests: 1,
    }
}

fn tag(response: &TestResponse) -> Result<String, Failure> {
    Ok(response
        .0
        .headers()
        .get(header::ETAG)
        .ok_or("an ETag")?
        .to_str()?
        .to_owned())
}

/// Every object key anywhere in the value.
fn keys(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, inner) in object {
                found.insert(key.clone());
                keys(inner, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                keys(item, found);
            }
        }
        _ => {}
    }
}

/// The groups of every bucket add up to the bucket, and each key's buckets add up to its
/// window group, so no group was dropped or merged.
fn assert_partitions(summary: &Summary, context: &str) -> Result<(), Failure> {
    let groups = summary.groups.as_ref().ok_or("groups when grouped")?;
    groups
        .iter()
        .fold(Metrics::default(), |sum, group| sum.plus(&group.metrics))
        .assert_same(&summary.total, context);
    let window_keys = groups
        .iter()
        .map(|group| group.key.clone())
        .collect::<Vec<_>>();
    let mut sorted = window_keys.clone();
    sorted.sort();
    assert_eq!(window_keys, sorted, "{context}: groups ordered by key");

    for bucket in &summary.buckets {
        let cells = bucket.groups.as_ref().ok_or("bucket groups when grouped")?;
        cells
            .iter()
            .fold(Metrics::default(), |sum, group| sum.plus(&group.metrics))
            .assert_same(&bucket.total, &format!("{context} at {}", bucket.start));
        assert!(
            cells.windows(2).all(|pair| pair[0].key < pair[1].key),
            "{context} at {}: bucket groups ordered by key",
            bucket.start
        );
        assert!(
            cells.iter().all(|cell| cell.metrics.requests > 0),
            "{context} at {}: only groups with usage",
            bucket.start
        );
    }
    for group in groups {
        summary
            .buckets
            .iter()
            .flat_map(|bucket| bucket.groups.iter().flatten())
            .filter(|cell| cell.key == group.key)
            .fold(Metrics::default(), |sum, cell| sum.plus(&cell.metrics))
            .assert_same(&group.metrics, &format!("{context} key {}", group.key));
    }

    Ok(())
}

#[tokio::test]
async fn the_window_totals_every_token_kind_call_and_stored_cost() -> Result<(), Failure> {
    let harness = harness().await?;

    let summary = harness.summary(&format!("{WINDOW}&step=hour")).await;

    summary.total.assert_same(&window_total(), "total");
    assert!(summary.groups.is_none());
    assert_eq!(summary.buckets.len(), 29);
    assert_eq!(summary.buckets[0].start, "2026-03-02T00:00:00Z");
    assert_eq!(summary.buckets[28].start, "2026-03-03T04:00:00Z");
    assert!(summary.buckets.iter().all(|bucket| bucket.groups.is_none()));
    let busy = [(10, 2), (11, 1), (23, 1), (24, 1), (28, 1)];
    for (index, bucket) in summary.buckets.iter().enumerate() {
        let expected = busy
            .iter()
            .find(|(busy, _)| *busy == index)
            .map_or(0, |(_, requests)| *requests);
        assert_eq!(bucket.total.requests, expected, "{}", bucket.start);
    }
    summary
        .buckets
        .iter()
        .fold(Metrics::default(), |sum, bucket| sum.plus(&bucket.total))
        .assert_same(&summary.total, "buckets");

    // a and b: a's oauth account is billed nothing.
    let ten = &summary.buckets[10].total;
    assert_eq!(ten.input_tokens, 1_200_000);
    assert_eq!(ten.uncached_input_tokens, 300_000);
    assert_eq!(ten.cache_read_tokens, 600_000);
    assert_eq!(ten.cache_write_tokens, 300_000);
    close(ten.list_cost_usd, 3.705);
    close(ten.billed_cost_usd, 0.75);
    // d has no price and no reported charge, so it is in neither cost.
    let late = &summary.buckets[23].total;
    assert_eq!(late.requests, 1);
    assert_eq!(late.reasoning_tokens, 1000);
    close(late.list_cost_usd, 0.0);
    close(late.billed_cost_usd, 0.0);
    assert_eq!(late.unpriced_requests, 1);
    assert_eq!(late.unbilled_requests, 1);
    // f has no price, but its gateway said what it charged.
    let last = &summary.buckets[28].total;
    close(last.list_cost_usd, 0.0);
    close(last.billed_cost_usd, 0.25);
    assert_eq!(last.unpriced_requests, 1);
    assert_eq!(last.unbilled_requests, 0);

    Ok(())
}

#[tokio::test]
async fn every_group_is_kept_and_the_groups_partition_the_totals() -> Result<(), Failure> {
    let harness = harness().await?;

    for step in ["hour", "day"] {
        for group in ["account", "model", "provider"] {
            let summary = harness
                .summary(&format!("{WINDOW}&step={step}&group={group}"))
                .await;
            summary.total.assert_same(&window_total(), group);
            assert_partitions(&summary, &format!("{step} by {group}"))?;
        }
    }

    let by_model = harness
        .summary(&format!("{WINDOW}&step=day&group=model"))
        .await;
    let models = by_model.groups.ok_or("model groups")?;
    assert_eq!(
        models
            .iter()
            .map(|group| group.key.as_str())
            .collect::<Vec<_>>(),
        [SONNET, CODEX]
    );
    let codex = &models[1].metrics;
    assert_eq!(codex.requests, 2);
    close(codex.list_cost_usd, 0.0);
    close(codex.billed_cost_usd, 0.25);
    assert_eq!(codex.unpriced_requests, 2);
    assert_eq!(codex.unbilled_requests, 1);
    let sonnet = &models[0].metrics;
    close(sonnet.list_cost_usd, 4.158);
    close(sonnet.billed_cost_usd, 1.203);
    assert_eq!(sonnet.unpriced_requests, 0);

    let by_provider = harness
        .summary(&format!("{WINDOW}&step=day&group=provider"))
        .await;
    assert_eq!(
        by_provider
            .groups
            .ok_or("provider groups")?
            .iter()
            .map(|group| group.key.as_str())
            .collect::<Vec<_>>(),
        ["claude", "codex"]
    );

    let by_account = harness
        .summary(&format!("{WINDOW}&step=day&group=account"))
        .await;
    let accounts = by_account.groups.ok_or("account groups")?;
    let mut expected = Vec::new();
    for (label, requests) in [("a1", 1), ("a2", 2), ("a3", 1), ("b1", 1), ("b2", 1)] {
        expected.push((harness.account(label).await?, requests));
    }
    expected.sort();
    assert_eq!(
        accounts
            .iter()
            .map(|group| (group.key.clone(), group.metrics.requests))
            .collect::<Vec<_>>(),
        expected,
        "`before` and `after` fall outside the window"
    );

    Ok(())
}

#[tokio::test]
async fn many_groups_are_never_merged_into_other() -> Result<(), Failure> {
    let harness = harness().await?;
    let requests = MODELS
        .into_iter()
        .map(|model| Request {
            upstream_id: model,
            at: "2026-04-01T12:00:00Z",
            model,
            input: 10,
            ..base()
        })
        .collect::<Vec<_>>();
    harness.ingest("alpha", &requests).await?;

    let summary = harness
        .summary("from=2026-04-01T00:00:00Z&to=2026-04-02T00:00:00Z&step=day&group=model")
        .await;

    let groups = summary.groups.as_ref().ok_or("model groups")?;
    assert_eq!(
        groups
            .iter()
            .map(|group| group.key.as_str())
            .collect::<Vec<_>>(),
        MODELS
    );
    assert_eq!(summary.buckets.len(), 1);
    assert_eq!(
        summary.buckets[0].groups.as_ref().map(Vec::len),
        Some(MODELS.len())
    );
    assert_partitions(&summary, "twelve models")?;

    Ok(())
}

#[tokio::test]
async fn buckets_follow_utc_boundaries_and_to_is_exclusive() -> Result<(), Failure> {
    let harness = harness().await?;

    let days = harness.summary(&format!("{WINDOW}&step=day")).await;
    assert_eq!(
        days.buckets
            .iter()
            .map(|bucket| (bucket.start.as_str(), bucket.total.requests))
            .collect::<Vec<_>>(),
        [("2026-03-02T00:00:00Z", 4), ("2026-03-03T00:00:00Z", 2)],
        "d at 23:59:59 closes 03-02 and e at midnight opens 03-03"
    );

    // An offset names the same instant: 01:00 at +01:00 is midnight UTC.
    let offset = harness
        .summary("from=2026-03-02T01:00:00%2B01:00&to=2026-03-03T05:00:00Z&step=day")
        .await;
    offset.total.assert_same(&days.total, "offset bounds");
    assert_eq!(offset.buckets[0].start, "2026-03-02T00:00:00Z");

    // A cut bucket keeps its whole-hour start and counts only what is inside: a at 10:00
    // is before `from`, b at 10:59:59.999 is in hour 10, c at 11:00 is `to` itself.
    let cut = harness
        .summary("from=2026-03-02T10:30:00Z&to=2026-03-02T11:00:00Z&step=hour")
        .await;
    assert_eq!(cut.buckets.len(), 1);
    assert_eq!(cut.buckets[0].start, "2026-03-02T10:00:00Z");
    assert_eq!(cut.buckets[0].total.requests, 1);
    assert_eq!(cut.buckets[0].total.input_tokens, 200_000);

    let past = harness
        .summary("from=2026-03-02T10:30:00Z&to=2026-03-02T11:00:01Z&step=hour")
        .await;
    assert_eq!(
        past.buckets
            .iter()
            .map(|bucket| (bucket.start.as_str(), bucket.total.requests))
            .collect::<Vec<_>>(),
        [("2026-03-02T10:00:00Z", 1), ("2026-03-02T11:00:00Z", 1)]
    );

    // A window with nothing in it still lists its buckets, zero filled.
    let empty = harness
        .summary("from=2026-02-01T00:00:00Z&to=2026-02-01T03:00:00Z&step=hour&group=account")
        .await;
    assert_eq!(empty.total.requests, 0);
    assert_eq!(empty.groups.as_ref().map(Vec::len), Some(0));
    assert_eq!(empty.buckets.len(), 3);
    assert!(
        empty.buckets.iter().all(|bucket| bucket.total.requests == 0
            && bucket.groups.as_ref().is_some_and(Vec::is_empty))
    );

    Ok(())
}

#[tokio::test]
async fn invalid_arguments_are_refused() -> Result<(), Failure> {
    let harness = harness().await?;

    for query in [
        "to=2026-03-03T00:00:00Z&step=hour",
        "from=2026-03-02T00:00:00Z&step=hour",
        "from=2026-03-02T00:00:00Z&to=2026-03-03T00:00:00Z",
        "from=2026-03-02T00:00:00Z&to=2026-03-03T00:00:00Z&step=week",
        "from=2026-03-02T00:00:00Z&to=2026-03-03T00:00:00Z&step=hour&group=harness",
        "from=2026-03-02T00:00:00Z&to=2026-03-03T00:00:00Z&step=hour&group=session",
    ] {
        harness.refused(query).await;
    }

    for (query, message) in [
        (
            "from=yesterday&to=2026-03-03T00:00:00Z&step=hour",
            "from must be an RFC 3339 timestamp",
        ),
        (
            "from=2026-03-02T00:00:00Z&to=2026-03-03&step=hour",
            "to must be an RFC 3339 timestamp",
        ),
        (
            "from=2026-03-02T00:00:00Z&to=2026-03-02T00:00:00Z&step=hour",
            "to must be after from",
        ),
        (
            "from=2026-03-03T00:00:00Z&to=2026-03-02T00:00:00Z&step=day",
            "to must be after from",
        ),
    ] {
        let response = harness.refused(query).await;
        let body = response.json().await;
        assert_eq!(body.value().object().get("message").string(), message);
    }

    // 8784 hours, past the 5000 buckets one answer may hold.
    let response = harness
        .refused("from=2020-01-01T00:00:00Z&to=2021-01-01T00:00:00Z&step=hour")
        .await;
    let body = response.json().await;
    assert!(
        body.value()
            .object()
            .get("message")
            .string()
            .contains("buckets")
    );
    // The same year in days is 366 buckets.
    let year = harness
        .summary("from=2020-01-01T00:00:00Z&to=2021-01-01T00:00:00Z&step=day")
        .await;
    assert_eq!(year.buckets.len(), 366);

    Ok(())
}

#[tokio::test]
async fn the_etag_holds_until_the_window_reads_differently() -> Result<(), Failure> {
    let harness = harness().await?;
    let path = format!("/api/usage/summary?{WINDOW}&step=day&group=model");

    let first = harness.client.get(&path).send().await;
    first.assert_status_is_ok();
    let current = tag(&first)?;
    for if_none_match in [current.clone(), format!("W/{current}"), "*".to_owned()] {
        let again = harness
            .client
            .get(&path)
            .header(header::IF_NONE_MATCH, &if_none_match)
            .send()
            .await;
        again.assert_status(StatusCode::NOT_MODIFIED);
        assert_eq!(tag(&again)?, current);
        again.assert_text("").await;
    }

    // An event outside the window leaves its answer, and so its tag, alone.
    harness
        .ingest(
            "alpha",
            &[Request {
                upstream_id: "outside",
                at: "2026-03-05T00:00:00Z",
                input: 10,
                ..base()
            }],
        )
        .await?;
    let unchanged = harness
        .client
        .get(&path)
        .header(header::IF_NONE_MATCH, &current)
        .send()
        .await;
    unchanged.assert_status(StatusCode::NOT_MODIFIED);

    // An event inside it changes the answer.
    harness
        .ingest(
            "alpha",
            &[Request {
                upstream_id: "inside",
                at: "2026-03-02T12:00:00Z",
                input: 10,
                ..base()
            }],
        )
        .await?;
    let grown = harness
        .client
        .get(&path)
        .header(header::IF_NONE_MATCH, &current)
        .send()
        .await;
    grown.assert_status_is_ok();
    let grown_tag = tag(&grown)?;
    assert_ne!(grown_tag, current);
    let summary = grown.json().await.value().deserialize::<Summary>();
    assert_eq!(summary.total.requests, 7);
    // `inside` is not priced until a fill runs.
    assert_eq!(summary.total.unpriced_requests, 3);

    // So does a price that turns an unknown cost into a known one: d and f at 1.25 in and
    // 10 out, with d's oauth account billed nothing and f keeping its reported charge.
    harness.price(CODEX, [1.25, 0.125, 1.25, 10.0]).await;
    ModelPrice::fill(&harness.state).await?;
    let priced = harness
        .client
        .get(&path)
        .header(header::IF_NONE_MATCH, &grown_tag)
        .send()
        .await;
    priced.assert_status_is_ok();
    assert_ne!(tag(&priced)?, grown_tag);
    let summary = priced.json().await.value().deserialize::<Summary>();
    close(summary.total.list_cost_usd, 4.158 + 0.00003 + 2.55 + 0.0725);
    close(summary.total.billed_cost_usd, 1.453);
    assert_eq!(summary.total.unpriced_requests, 0);
    assert_eq!(summary.total.unbilled_requests, 0);

    Ok(())
}

#[tokio::test]
async fn the_consumer_token_reads_only_aggregate_fields() -> Result<(), Failure> {
    let harness = harness().await?;
    let path = format!("/api/usage/summary?{WINDOW}&step=hour&group=account");

    let anonymous = harness.anonymous.get(&path).send().await;
    anonymous.assert_status(StatusCode::UNAUTHORIZED);

    let response = harness
        .anonymous
        .get(&path)
        .header(header::AUTHORIZATION, format!("Bearer {CONSUMER_TOKEN}"))
        .send()
        .await;
    response.assert_status_is_ok();
    tag(&response)?;
    let body = response.json().await.value().deserialize::<Value>();
    let mut found = BTreeSet::new();
    keys(&body, &mut found);
    let allowed = [
        "total",
        "groups",
        "buckets",
        "start",
        "key",
        "metrics",
        "requests",
        "input_tokens",
        "uncached_input_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "output_tokens",
        "reasoning_tokens",
        "unclassified_tokens",
        "total_tokens",
        "list_cost_usd",
        "billed_cost_usd",
        "unpriced_requests",
        "unbilled_requests",
    ]
    .map(str::to_owned)
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(found, allowed);

    let text = body.to_string();
    for detail in [
        "harness-omp",
        "a session",
        "sk-client-masked",
        "an agent/1.0",
        "POST /v1/messages",
        "upstream overloaded",
    ] {
        assert!(!text.contains(detail), "the answer names {detail}");
    }

    Ok(())
}
