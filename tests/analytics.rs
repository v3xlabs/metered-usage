use std::sync::Arc;

use jiff::Timestamp;
use metered_usage::account::{AuthKind, NewAccount};
use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::price::ModelPrice;
use metered_usage::source::Source;
use metered_usage::usage::NewUsageEvent;
use metered_usage::usage::ingest::{Incoming, ingest};
use poem::Endpoint;
use poem::http::{StatusCode, header};
use poem::test::TestClient;
use serde::Deserialize;
use serde_json::json;

const TOKEN: &str = "correct horse battery staple";
const TOLERANCE: f64 = 1e-9;
const SONNET: &str = "claude-sonnet-4-5";
const CODEX: &str = "gpt-5-codex";
const WEEK: &str = "from=2026-03-01T00:00:00Z&to=2026-03-08T00:00:00Z";

type Failure = Box<dyn std::error::Error>;

struct Harness<E> {
    client: TestClient<E>,
}

#[derive(Debug, Deserialize)]
struct Metrics {
    requests: i64,
    failures: i64,
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
    cache_savings_usd: f64,
    unpriced_requests: i64,
    avg_latency_ms: Option<f64>,
    avg_ttft_ms: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Summary {
    metrics: Metrics,
    range_days: i64,
    active_days: i64,
    daily_burn: DailyBurn,
    peak_day_by_cost: Option<PeakDay>,
    peak_day_by_tokens: Option<PeakDay>,
    cache_hit_rate: Option<f64>,
    distinct: Distinct,
}

#[derive(Debug, Deserialize)]
struct DailyBurn {
    list_cost_usd: f64,
    billed_cost_usd: f64,
    total_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct PeakDay {
    day: String,
    list_cost_usd: f64,
    total_tokens: i64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Distinct {
    models: i64,
    providers: i64,
    accounts: i64,
    harnesses: i64,
    sources: i64,
    sessions: i64,
}

#[derive(Debug, Deserialize)]
struct Series {
    buckets: Vec<SeriesBucket>,
}

#[derive(Debug, Deserialize)]
struct SeriesBucket {
    start: String,
    key: String,
    metrics: Metrics,
}

#[derive(Debug, Deserialize)]
struct Breakdown {
    rows: Vec<BreakdownRow>,
    total: Metrics,
}

#[derive(Debug, Deserialize)]
struct BreakdownRow {
    key: String,
    metrics: Metrics,
}

#[derive(Debug, Deserialize)]
struct SessionList {
    sessions: Vec<SessionRow>,
}

#[derive(Debug, Deserialize)]
struct SessionRow {
    session_id: String,
    source_id: String,
    account: Account,
    harness: Option<String>,
    models: Vec<String>,
    first_at: String,
    last_at: String,
    metrics: Metrics,
}

#[derive(Debug, Deserialize)]
struct Dimensions {
    models: Vec<String>,
    providers: Vec<String>,
    harnesses: Vec<String>,
    accounts: Vec<Account>,
    sources: Vec<SourceRef>,
}

#[derive(Debug, Deserialize)]
struct Account {
    account_id: String,
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountList {
    accounts: Vec<Account>,
}

#[derive(Debug, Deserialize)]
struct SourceRef {
    source_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct SourceList {
    sources: Vec<ListedSource>,
}

#[derive(Debug, Deserialize)]
struct ListedSource {
    source_id: String,
    key: String,
}

/// One request, as `request` below describes it before the defaults are filled in.
#[derive(Clone, Copy)]
struct Request {
    upstream_id: &'static str,
    at: &'static str,
    model: &'static str,
    provider: &'static str,
    account: (&'static str, AuthKind),
    harness: Option<&'static str>,
    session: Option<&'static str>,
    input: i64,
    cache_read: i64,
    cache_write: i64,
    output: i64,
    latency_ms: Option<i64>,
    ttft_ms: Option<i64>,
    failed: bool,
}

impl<E: Endpoint> Harness<E> {
    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> T {
        let response = self.client.get(path).send().await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize()
    }

    async fn account(&self, label: &str) -> Result<String, Failure> {
        self.get::<AccountList>("/api/accounts")
            .await
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

    async fn source(&self, key: &str) -> Result<String, Failure> {
        self.get::<SourceList>("/api/sources")
            .await
            .sources
            .into_iter()
            .find(|source| source.key == key)
            .map(|source| source.source_id)
            .ok_or_else(|| format!("no source {key}").into())
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
        caller: None,
        harness: request.harness.map(str::to_owned),
        user_agent: None,
        session_id: request.session.map(str::to_owned),
        input_tokens: request.input,
        output_tokens: request.output,
        reasoning_tokens: 0,
        cache_read_tokens: request.cache_read,
        cache_write_tokens: request.cache_write,
        unclassified_tokens: 0,
        total_tokens: request.input + request.output,
        token_quality: None,
        latency_ms: request.latency_ms,
        ttft_ms: request.ttft_ms,
        streamed: false,
        status_code: if request.failed { 500 } else { 200 },
        failed: request.failed,
        error_message: None,
        service_tier: None,
        reasoning_effort: None,
        billed_cost_usd: None,
    })
}

/// Sonnet is priced by hand at 3 in, 0.30 cached, 3.75 cache write and 15 out per million;
/// Codex has no price. 2026-03-02 is a Monday.
///
/// | event | at (UTC)          | source | account | harness | session | in        | read    | write   | out     | list  | billed | savings |
/// | a     | 03-02 10:00       | alpha  | a1 oauth | omp    | s1      | 1 000 000 | 600 000 | 100 000 | 100 000 | 2.955 | 0      | 1.545   |
/// | b     | 03-02 12:00       | alpha  | a2 key  | omp     | s1      | 200 000   | 0       | 200 000 | 0       | 0.75  | 0.75   | -0.15   |
/// | c     | 03-02 13:00       | alpha  | a2 key  | omp     | s1      | 1 000     | 0       | 0       | 0       | 0.003 | 0.003  | 0       |
/// | d     | 03-04 08:00 fails | alpha  | a3 oauth | none   | none    | 2 000 000 | 0       | 0       | 5 000   | none  | none   | none    |
/// | e     | 03-04 23:30       | beta   | b1 key  | cc      | s2      | 100 000   | 0       | 0       | 10 000  | 0.45  | 0.45   | 0       |
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
    let token = Token::try_from(TOKEN.to_owned())?;
    let state = Arc::new(AppState::new(database, token, config)?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));

    let created = client
        .post("/api/prices")
        .body_json(&json!({
            "model": SONNET,
            "effective_from": "2025-01-01T00:00:00Z",
            "input_usd_per_mtok": 3.0,
            "cached_input_usd_per_mtok": 0.3,
            "cache_write_usd_per_mtok": 3.75,
            "output_usd_per_mtok": 15.0,
        }))
        .send()
        .await;
    created.assert_status_is_ok();

    for (key, requests) in requests() {
        let source = Source::by_key(&state.database, key)
            .await?
            .ok_or("a configured source")?;
        let records = requests
            .iter()
            .map(|each| request(each).map(|event| Incoming::Record(Box::new(event))))
            .collect::<Result<Vec<_>, _>>()?;
        let report = ingest(&state, source.id, records).await?;
        assert_eq!(report.accepted, i64::try_from(requests.len())?);
    }
    ModelPrice::fill(&state).await?;

    Ok(Harness { client })
}

/// The events of the table on [`harness`].
fn requests() -> [(&'static str, Vec<Request>); 2] {
    let base = Request {
        upstream_id: "",
        at: "",
        model: SONNET,
        provider: "claude",
        account: ("a1", AuthKind::OAuth),
        harness: Some("omp"),
        session: Some("s1"),
        input: 0,
        cache_read: 0,
        cache_write: 0,
        output: 0,
        latency_ms: None,
        ttft_ms: None,
        failed: false,
    };
    let alpha = vec![
        Request {
            upstream_id: "a",
            at: "2026-03-02T10:00:00Z",
            input: 1_000_000,
            cache_read: 600_000,
            cache_write: 100_000,
            output: 100_000,
            latency_ms: Some(1000),
            ttft_ms: Some(200),
            ..base
        },
        Request {
            upstream_id: "b",
            at: "2026-03-02T12:00:00Z",
            account: ("a2", AuthKind::ApiKey),
            input: 200_000,
            cache_write: 200_000,
            latency_ms: Some(3000),
            ..base
        },
        Request {
            upstream_id: "c",
            at: "2026-03-02T13:00:00Z",
            account: ("a2", AuthKind::ApiKey),
            input: 1000,
            ..base
        },
        Request {
            upstream_id: "d",
            at: "2026-03-04T08:00:00Z",
            model: CODEX,
            provider: "codex",
            account: ("a3", AuthKind::OAuth),
            harness: None,
            session: None,
            input: 2_000_000,
            output: 5000,
            failed: true,
            ..base
        },
    ];
    let beta = vec![Request {
        upstream_id: "e",
        at: "2026-03-04T23:30:00Z",
        account: ("b1", AuthKind::ApiKey),
        harness: Some("cc"),
        session: Some("s2"),
        input: 100_000,
        output: 10_000,
        ..base
    }];
    [("alpha", alpha), ("beta", beta)]
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{actual} is not {expected}"
    );
}

#[tokio::test]
async fn a_summary_reports_every_measure_of_the_window() -> Result<(), Failure> {
    let harness = harness().await?;

    let summary = harness
        .get::<Summary>(&format!("/api/analytics/summary?{WEEK}"))
        .await;

    let metrics = &summary.metrics;
    assert_eq!(metrics.requests, 5);
    assert_eq!(metrics.failures, 1);
    assert_eq!(metrics.input_tokens, 3_301_000);
    assert_eq!(metrics.uncached_input_tokens, 2_401_000);
    assert_eq!(metrics.cache_read_tokens, 600_000);
    assert_eq!(metrics.cache_write_tokens, 300_000);
    assert_eq!(metrics.output_tokens, 115_000);
    assert_eq!(metrics.reasoning_tokens, 0);
    assert_eq!(metrics.unclassified_tokens, 0);
    assert_eq!(metrics.total_tokens, 3_416_000);
    close(metrics.list_cost_usd, 4.158);
    close(metrics.billed_cost_usd, 1.203);
    close(metrics.cache_savings_usd, 1.395);
    assert_eq!(metrics.unpriced_requests, 1);
    close(metrics.avg_latency_ms.ok_or("a latency")?, 2000.0);
    close(metrics.avg_ttft_ms.ok_or("a time to first token")?, 200.0);

    assert_eq!(summary.range_days, 7);
    assert_eq!(summary.active_days, 2);
    close(summary.daily_burn.list_cost_usd, 4.158 / 7.0);
    close(summary.daily_burn.billed_cost_usd, 1.203 / 7.0);
    assert_eq!(summary.daily_burn.total_tokens, 488_000);
    let by_cost = summary.peak_day_by_cost.ok_or("a peak day by cost")?;
    assert_eq!(by_cost.day, "2026-03-02");
    close(by_cost.list_cost_usd, 3.708);
    assert_eq!(by_cost.total_tokens, 1_301_000);
    let by_tokens = summary.peak_day_by_tokens.ok_or("a peak day by tokens")?;
    assert_eq!(by_tokens.day, "2026-03-04");
    close(by_tokens.list_cost_usd, 0.45);
    assert_eq!(by_tokens.total_tokens, 2_115_000);
    close(
        summary.cache_hit_rate.ok_or("a cache hit rate")?,
        600_000.0 / 3_301_000.0,
    );
    assert_eq!(
        summary.distinct,
        Distinct {
            models: 2,
            providers: 2,
            accounts: 4,
            harnesses: 3,
            sources: 2,
            sessions: 2,
        }
    );

    Ok(())
}

#[tokio::test]
async fn an_empty_window_has_no_peak_and_no_hit_rate() -> Result<(), Failure> {
    let harness = harness().await?;

    let summary = harness
        .get::<Summary>("/api/analytics/summary?from=2026-02-01T00:00:00Z&to=2026-02-03T00:00:00Z")
        .await;

    assert_eq!(summary.metrics.requests, 0);
    assert_eq!(summary.range_days, 2);
    assert_eq!(summary.active_days, 0);
    assert!(summary.peak_day_by_cost.is_none());
    assert!(summary.peak_day_by_tokens.is_none());
    assert!(summary.cache_hit_rate.is_none());
    assert!(summary.metrics.avg_latency_ms.is_none());

    Ok(())
}

#[tokio::test]
async fn cache_writes_that_outweigh_reads_save_a_negative_amount() -> Result<(), Failure> {
    let harness = harness().await?;
    let a2 = harness.account("a2").await?;

    let breakdown = harness
        .get::<Breakdown>(&format!("/api/analytics/breakdown?group_by=account&{WEEK}"))
        .await;

    let row = breakdown
        .rows
        .iter()
        .find(|row| row.key == a2)
        .ok_or("the row of a2")?;
    close(row.metrics.cache_savings_usd, -0.15);

    Ok(())
}

#[tokio::test]
async fn a_local_day_follows_the_viewers_offset() -> Result<(), Failure> {
    let harness = harness().await?;

    let utc = harness
        .get::<Series>(&format!("/api/analytics/series?bucket=day&{WEEK}"))
        .await;
    let local = harness
        .get::<Series>(&format!(
            "/api/analytics/series?bucket=day&utc_offset_minutes=120&{WEEK}"
        ))
        .await;

    let requests = |series: &Series, start: &str| {
        series
            .buckets
            .iter()
            .find(|bucket| bucket.start == start)
            .map(|bucket| bucket.metrics.requests)
    };
    assert_eq!(utc.buckets.len(), 7);
    assert_eq!(requests(&utc, "2026-03-04T00:00:00+00:00"), Some(2));
    assert_eq!(requests(&utc, "2026-03-05T00:00:00+00:00"), Some(0));
    // Local 2026-03-01 starts two hours before the window, and local 2026-03-08 starts
    // two hours before it ends, so both are part of it.
    assert_eq!(local.buckets.len(), 8);
    assert_eq!(
        local.buckets.first().map(|bucket| bucket.start.as_str()),
        Some("2026-03-01T00:00:00+02:00")
    );
    assert_eq!(requests(&local, "2026-03-04T00:00:00+02:00"), Some(1));
    assert_eq!(
        requests(&local, "2026-03-05T00:00:00+02:00"),
        Some(1),
        "23:30 UTC is 01:30 the next day"
    );

    let local_summary = harness
        .get::<Summary>(&format!(
            "/api/analytics/summary?utc_offset_minutes=120&{WEEK}"
        ))
        .await;
    assert_eq!(local_summary.range_days, 8);
    assert_eq!(local_summary.active_days, 3);

    Ok(())
}

#[tokio::test]
async fn every_bucket_of_the_window_is_present_and_zero_filled() -> Result<(), Failure> {
    let harness = harness().await?;

    let series = harness
        .get::<Series>(&format!("/api/analytics/series?bucket=day&{WEEK}"))
        .await;

    let starts = series
        .buckets
        .iter()
        .map(|bucket| bucket.start.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        starts,
        [
            "2026-03-01T00:00:00+00:00",
            "2026-03-02T00:00:00+00:00",
            "2026-03-03T00:00:00+00:00",
            "2026-03-04T00:00:00+00:00",
            "2026-03-05T00:00:00+00:00",
            "2026-03-06T00:00:00+00:00",
            "2026-03-07T00:00:00+00:00",
        ]
    );
    assert!(series.buckets.iter().all(|bucket| bucket.key == "all"));
    let empty = &series.buckets[0].metrics;
    assert_eq!((empty.requests, empty.total_tokens), (0, 0));
    close(empty.list_cost_usd, 0.0);
    assert!(empty.avg_latency_ms.is_none());

    let weeks = harness
        .get::<Series>(&format!("/api/analytics/series?bucket=week&{WEEK}"))
        .await;
    let weeks = weeks
        .buckets
        .iter()
        .map(|bucket| (bucket.start.as_str(), bucket.metrics.requests))
        .collect::<Vec<_>>();
    assert_eq!(
        weeks,
        [
            ("2026-02-23T00:00:00+00:00", 0),
            ("2026-03-02T00:00:00+00:00", 5),
        ],
        "a week starts on the Monday"
    );

    Ok(())
}

#[tokio::test]
async fn groups_outside_the_top_merge_into_other() -> Result<(), Failure> {
    let harness = harness().await?;
    let (a1, a3) = (harness.account("a1").await?, harness.account("a3").await?);

    let series = harness
        .get::<Series>(&format!(
            "/api/analytics/series?bucket=day&group_by=account&top=2&rank_by=total_tokens&{WEEK}"
        ))
        .await;

    assert_eq!(series.buckets.len(), 7 * 3, "every key has every bucket");
    let keys = series.buckets[..3]
        .iter()
        .map(|bucket| bucket.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(keys, [a3.clone(), a1.clone(), "other".to_owned()]);
    let other = series
        .buckets
        .iter()
        .filter(|bucket| bucket.key == "other")
        .fold((0, 0), |(requests, tokens), bucket| {
            (
                requests + bucket.metrics.requests,
                tokens + bucket.metrics.total_tokens,
            )
        });
    assert_eq!(other, (3, 311_000), "a2 and b1");
    let on = |key: &str, start: &str| {
        series
            .buckets
            .iter()
            .find(|bucket| bucket.key == key && bucket.start == start)
            .map(|bucket| bucket.metrics.requests)
    };
    assert_eq!(on("other", "2026-03-02T00:00:00+00:00"), Some(2));
    assert_eq!(on(&a3, "2026-03-02T00:00:00+00:00"), Some(0));

    Ok(())
}

#[tokio::test]
async fn values_of_one_filter_are_alternatives_and_filters_are_all_required() -> Result<(), Failure>
{
    let harness = harness().await?;
    let requests = async |filters: &str| {
        harness
            .get::<Summary>(&format!("/api/analytics/summary?{WEEK}&{filters}"))
            .await
            .metrics
            .requests
    };

    assert_eq!(requests(&format!("model={SONNET}")).await, 4);
    assert_eq!(requests(&format!("model={SONNET},{CODEX}")).await, 5);
    assert_eq!(
        requests(&format!("model={SONNET},{CODEX}&harness=omp")).await,
        3
    );
    assert_eq!(requests("harness=unknown").await, 1);
    assert_eq!(requests("harness=unknown,cc").await, 2);
    assert_eq!(requests(&format!("provider=claude&model={CODEX}")).await, 0);
    let (a2, beta) = (harness.account("a2").await?, harness.source("beta").await?);
    assert_eq!(requests(&format!("account_id={a2}")).await, 2);
    assert_eq!(requests(&format!("source_id={beta}")).await, 1);
    assert_eq!(
        requests(&format!("source_id={beta}&account_id={a2}")).await,
        0
    );

    Ok(())
}

#[tokio::test]
async fn a_breakdown_ranks_every_group_and_totals_past_its_limit() -> Result<(), Failure> {
    let harness = harness().await?;
    let (a1, a2) = (harness.account("a1").await?, harness.account("a2").await?);

    let breakdown = harness
        .get::<Breakdown>(&format!(
            "/api/analytics/breakdown?group_by=account&limit=2&{WEEK}"
        ))
        .await;

    let keys = breakdown
        .rows
        .iter()
        .map(|row| row.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(keys, [a1, a2], "by list cost: 2.955, then 0.753");
    assert_eq!(breakdown.total.requests, 5);

    let harnesses = harness
        .get::<Breakdown>(&format!(
            "/api/analytics/breakdown?group_by=harness&rank_by=requests&{WEEK}"
        ))
        .await;
    let rows = harnesses
        .rows
        .iter()
        .map(|row| (row.key.as_str(), row.metrics.requests))
        .collect::<Vec<_>>();
    assert_eq!(rows, [("omp", 3), ("cc", 1), ("unknown", 1)]);

    Ok(())
}

#[tokio::test]
async fn a_session_is_ranked_and_reported_under_its_busiest_account() -> Result<(), Failure> {
    let harness = harness().await?;
    let (a2, alpha) = (harness.account("a2").await?, harness.source("alpha").await?);

    let sessions = harness
        .get::<SessionList>(&format!(
            "/api/analytics/sessions?rank_by=total_tokens&{WEEK}"
        ))
        .await
        .sessions;

    let ids = sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        ["s1", "s2"],
        "an event without a session is no session"
    );
    let s1 = &sessions[0];
    assert_eq!(
        s1.account.account_id, a2,
        "a2 carried two of its three requests"
    );
    assert_eq!(s1.source_id, alpha);
    assert_eq!(s1.harness.as_deref(), Some("omp"));
    assert_eq!(s1.models, [SONNET]);
    assert_eq!(s1.first_at, "2026-03-02T10:00:00Z");
    assert_eq!(s1.last_at, "2026-03-02T13:00:00Z");
    assert_eq!(s1.metrics.requests, 3);
    assert_eq!(s1.metrics.total_tokens, 1_301_000);

    let limited = harness
        .get::<SessionList>(&format!(
            "/api/analytics/sessions?rank_by=requests&limit=1&{WEEK}"
        ))
        .await
        .sessions;
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].session_id, "s1");

    Ok(())
}

#[tokio::test]
async fn dimensions_list_only_what_the_window_holds() -> Result<(), Failure> {
    let harness = harness().await?;
    let beta = harness.source("beta").await?;

    let dimensions = harness
        .get::<Dimensions>(
            "/api/analytics/dimensions?from=2026-03-04T12:00:00Z&to=2026-03-08T00:00:00Z",
        )
        .await;

    assert_eq!(dimensions.models, [SONNET]);
    assert_eq!(dimensions.providers, ["claude"]);
    assert_eq!(dimensions.harnesses, ["cc"]);
    let b1 = harness.account("b1").await?;
    let accounts = dimensions
        .accounts
        .iter()
        .map(|account| account.account_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(accounts, [b1.as_str()]);
    assert_eq!(dimensions.sources.len(), 1);
    assert_eq!(dimensions.sources[0].source_id, beta);
    assert_eq!(dimensions.sources[0].name, "beta");

    let everything = harness
        .get::<Dimensions>(&format!("/api/analytics/dimensions?{WEEK}"))
        .await;
    assert_eq!(everything.harnesses, ["cc", "omp", "unknown"]);
    assert_eq!(everything.accounts.len(), 4);

    Ok(())
}

#[tokio::test]
async fn an_id_that_names_nothing_is_refused() -> Result<(), Failure> {
    let harness = harness().await?;

    for query in [
        "account_id=not-an-id",
        "source_id=0000000000",
        "account_id=00000000000",
        "source_id=00000000000",
        "utc_offset_minutes=100000",
    ] {
        let response = harness
            .client
            .get(format!("/api/analytics/summary?{query}"))
            .send()
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }
    let too_long = harness
        .client
        .get("/api/analytics/series?bucket=hour&from=2020-01-01T00:00:00Z&to=2026-01-01T00:00:00Z")
        .send()
        .await;
    too_long.assert_status(StatusCode::BAD_REQUEST);

    Ok(())
}
