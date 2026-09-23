use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::source::Source as StoredSource;
use poem::Endpoint;
use poem::http::{StatusCode, header};
use poem::test::TestClient;
use serde::Deserialize;
use serde_json::{Value, json};

const ANTIGRAVITY: &str = include_str!("fixtures/antigravity.json");
const CLAUDE: &str = include_str!("fixtures/claude.json");
const GROK: &str = include_str!("fixtures/grok.json");
const TOKEN: &str = "correct horse battery staple";
const API_KEY: &str = "sk-test-0123456789abcd";
const CONFIG: &str = r#"
    [[source]]
    key = "workstation"
    kind = "cliproxy"
    base_url = "http://127.0.0.1:8317/"
    key_env = "WORKSTATION_MANAGEMENT_KEY"
"#;

#[derive(Deserialize)]
struct SourceList {
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    #[serde(rename = "source_id")]
    id: String,
    key: String,
    base_url: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct IngestResult {
    accepted: i64,
    duplicates: i64,
    rejected: i64,
}

#[derive(Deserialize)]
struct EventPage {
    events: Vec<UsageEvent>,
}

#[derive(Deserialize)]
struct UsageEvent {
    event_id: String,
    model: String,
    provider: String,
    account: AccountSummary,
    caller: Option<String>,
    harness: Option<String>,
    user_agent: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    unclassified_tokens: i64,
    total_tokens: i64,
    token_quality: Option<String>,
    status_code: i64,
    failed: bool,
    error_message: Option<String>,
}

#[derive(Deserialize)]
struct AccountSummary {
    account_id: String,
    label: Option<String>,
}

#[derive(Deserialize)]
struct AccountList {
    accounts: Vec<ListedAccount>,
}

#[derive(Deserialize)]
struct ListedAccount {
    account_id: String,
    provider: String,
    label: Option<String>,
    merged_into_account_id: Option<String>,
    event_count: i64,
}

#[derive(Deserialize)]
struct Summary {
    metrics: Metrics,
    distinct: Distinct,
}

#[derive(Deserialize)]
struct Metrics {
    requests: i64,
    failures: i64,
    input_tokens: i64,
    output_tokens: i64,
    unclassified_tokens: i64,
    total_tokens: i64,
    unpriced_requests: i64,
}

#[derive(Deserialize)]
struct Distinct {
    accounts: i64,
}

#[derive(Deserialize)]
struct Breakdown {
    rows: Vec<BreakdownRow>,
}

#[derive(Deserialize)]
struct BreakdownRow {
    key: String,
    metrics: Metrics,
}

#[derive(Deserialize)]
struct Series {
    buckets: Vec<SeriesBucket>,
}

#[derive(Deserialize)]
struct SeriesBucket {
    start: String,
    key: String,
    metrics: Metrics,
}

#[derive(Deserialize)]
struct DeadLetterList {
    dead_letters: Vec<DeadLetter>,
}

#[derive(Deserialize)]
struct DeadLetter {
    source_id: String,
    error: String,
    payload: String,
}

struct Gateway<E> {
    /// Presents the token on every request.
    client: TestClient<E>,
    /// Presents nothing it is not told to.
    anonymous: TestClient<E>,
    source_id: String,
}

async fn gateway() -> Result<Gateway<impl Endpoint>, Box<dyn std::error::Error>> {
    let config = Config::parse(CONFIG, |variable| {
        (variable == "WORKSTATION_MANAGEMENT_KEY").then(|| "management key".to_owned())
    })?;
    let database = Database::open("sqlite::memory:", 0).await?;
    StoredSource::sync(&database, &config).await?;
    let token = Token::try_from(TOKEN.to_owned())?;
    let state = Arc::new(AppState::new(database, token, config)?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let anonymous = TestClient::new(metered_usage::http::routes(&state));
    let listed = client.get("/api/sources").send().await;
    listed.assert_status_is_ok();
    let mut sources = listed
        .json()
        .await
        .value()
        .deserialize::<SourceList>()
        .sources;
    assert_eq!(sources.len(), 1, "exactly the configured source");
    let source = sources.remove(0);
    assert_eq!(source.key, "workstation");
    assert_eq!(source.base_url, "http://127.0.0.1:8317");
    assert!(source.enabled);

    Ok(Gateway {
        client,
        anonymous,
        source_id: source.id,
    })
}

/// Every reference record names its credential `==REDACTED==`, so each is given its own.
fn record(text: &str, auth_index: &str) -> Result<Value, serde_json::Error> {
    let mut record = serde_json::from_str::<Value>(text)?;
    record["auth_index"] = json!(auth_index);

    Ok(record)
}

fn batch() -> Result<Value, serde_json::Error> {
    Ok(json!({
        "records": [
            record(ANTIGRAVITY, "antigravity-1")?,
            record(CLAUDE, "claude-1")?,
            record(GROK, "xai-1")?,
        ],
    }))
}

async fn post_batch(
    client: &TestClient<impl Endpoint>,
    source_id: &str,
    batch: &Value,
) -> IngestResult {
    let posted = client
        .post(format!("/api/sources/{source_id}/events"))
        .body_json(batch)
        .send()
        .await;
    posted.assert_status_is_ok();

    posted.json().await.value().deserialize::<IngestResult>()
}

async fn body_of(
    client: &TestClient<impl Endpoint>,
    path: &str,
) -> Result<String, poem::error::ReadBodyError> {
    let response = client.get(path).send().await;
    response.assert_status_is_ok();

    response.0.into_body().into_string().await
}

async fn events(client: &TestClient<impl Endpoint>) -> Vec<UsageEvent> {
    let listed = client.get("/api/usage/events").send().await;
    listed.assert_status_is_ok();

    listed
        .json()
        .await
        .value()
        .deserialize::<EventPage>()
        .events
}

async fn accounts(client: &TestClient<impl Endpoint>) -> Vec<ListedAccount> {
    let listed = client.get("/api/accounts").send().await;
    listed.assert_status_is_ok();

    listed
        .json()
        .await
        .value()
        .deserialize::<AccountList>()
        .accounts
}

async fn summary(client: &TestClient<impl Endpoint>) -> Summary {
    let summary = client.get("/api/analytics/summary").send().await;
    summary.assert_status_is_ok();

    summary.json().await.value().deserialize::<Summary>()
}

#[tokio::test]
async fn a_batch_lands_as_the_gateway_described_it() {
    let gateway = gateway().await.expect("a gateway");
    let batch = batch().expect("the reference records");

    let ingested = post_batch(&gateway.client, &gateway.source_id, &batch).await;
    assert_eq!(ingested.accepted, 3);
    assert_eq!(ingested.duplicates, 0);
    assert_eq!(ingested.rejected, 0);

    let events = events(&gateway.client).await;
    assert_eq!(events.len(), 3);

    let antigravity = events
        .iter()
        .find(|event| event.provider == "antigravity")
        .expect("the antigravity record");
    assert_eq!(antigravity.model, "gpt-oss-120b-medium");
    assert_eq!(antigravity.input_tokens, 12000);
    assert_eq!(antigravity.output_tokens, 345);
    assert_eq!(antigravity.total_tokens, 12345);
    assert!(!antigravity.failed);
    assert_eq!(antigravity.status_code, 200);
    assert_eq!(antigravity.error_message, None);
    assert_eq!(antigravity.harness.as_deref(), Some("omp"));
    assert_eq!(
        antigravity.account.label.as_deref(),
        Some("person@example.com")
    );
    assert_eq!(antigravity.user_agent.as_deref(), Some("omp/1.0.0"));
    assert_eq!(antigravity.token_quality.as_deref(), Some("complete"));

    let claude = events
        .iter()
        .find(|event| event.provider == "claude")
        .expect("the claude record");
    assert!(claude.failed);
    assert_eq!(claude.status_code, 403);
    assert!(
        claude
            .error_message
            .as_deref()
            .is_some_and(|body| body.contains("You've reached your 5-hour usage limit")),
        "the upstream error body is kept: {:?}",
        claude.error_message
    );

    let summary = summary(&gateway.client).await;
    assert_eq!(summary.metrics.requests, 3);
    assert_eq!(summary.metrics.failures, 2);
    assert_eq!(summary.metrics.input_tokens, 12000);
    assert_eq!(summary.metrics.output_tokens, 345);
    assert_eq!(summary.metrics.total_tokens, 12345);
    assert_eq!(summary.metrics.unclassified_tokens, 0);
    assert_eq!(
        summary.metrics.unpriced_requests, 3,
        "no price is known in this test"
    );
    assert_eq!(summary.distinct.accounts, 3);
}

#[tokio::test]
async fn an_account_is_the_credential_and_not_the_email() {
    let gateway = gateway().await.expect("a gateway");
    post_batch(
        &gateway.client,
        &gateway.source_id,
        &batch().expect("the reference records"),
    )
    .await;

    let fixtures = accounts(&gateway.client).await;
    assert_eq!(fixtures.len(), 3, "claude, xai and antigravity differ");
    let original = fixtures
        .iter()
        .find(|account| account.provider == "antigravity")
        .expect("the antigravity account");
    assert_eq!(original.label.as_deref(), Some("person@example.com"));

    let second_subscription = record(ANTIGRAVITY, "antigravity-2").expect("a reference record");
    let ingested = post_batch(
        &gateway.client,
        &gateway.source_id,
        &json!({ "records": [second_subscription] }),
    )
    .await;
    assert_eq!(
        ingested.accepted, 1,
        "another credential is another request"
    );

    let antigravity = accounts(&gateway.client)
        .await
        .into_iter()
        .filter(|account| account.provider == "antigravity")
        .collect::<Vec<_>>();
    assert_eq!(antigravity.len(), 2);
    assert!(antigravity.iter().all(|account| account.label.as_deref()
        == Some("person@example.com")
        && account.event_count == 1));
    assert!(
        antigravity
            .iter()
            .any(|account| account.account_id == original.account_id)
    );
}

#[tokio::test]
async fn an_api_key_is_never_shown() {
    let gateway = gateway().await.expect("a gateway");
    let mut keyed = record(CLAUDE, "0b7e44d2").expect("a reference record");
    keyed["auth_type"] = json!("apikey");
    keyed["source"] = json!(API_KEY);
    keyed["api_key"] = json!(API_KEY);
    post_batch(
        &gateway.client,
        &gateway.source_id,
        &json!({ "records": [keyed] }),
    )
    .await;

    let events_body = body_of(&gateway.client, "/api/usage/events")
        .await
        .expect("an event page");
    let accounts_body = body_of(&gateway.client, "/api/accounts")
        .await
        .expect("an account list");
    assert!(!events_body.contains(API_KEY), "{events_body}");
    assert!(!accounts_body.contains(API_KEY), "{accounts_body}");

    let accounts = serde_json::from_str::<AccountList>(&accounts_body)
        .expect("an account list")
        .accounts;
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].label.as_deref(), Some("…abcd"));
    let events = serde_json::from_str::<EventPage>(&events_body)
        .expect("an event page")
        .events;
    assert_eq!(events[0].account.label.as_deref(), Some("…abcd"));
    assert_eq!(events[0].caller.as_deref(), Some("…abcd"));
    assert_eq!(events[0].account.account_id, accounts[0].account_id);
}

#[tokio::test]
async fn two_requests_that_share_a_request_id_are_two_events() {
    let gateway = gateway().await.expect("a gateway");
    let mut first = record(ANTIGRAVITY, "antigravity-1").expect("a reference record");
    first["request_id"] = json!("00000000");
    let mut second = first.clone();
    second["latency_ms"] = json!(1128);

    let ingested = post_batch(
        &gateway.client,
        &gateway.source_id,
        &json!({ "records": [first, second] }),
    )
    .await;
    assert_eq!(ingested.accepted, 2);
    assert_eq!(ingested.duplicates, 0);
    assert_eq!(events(&gateway.client).await.len(), 2);
}

#[tokio::test]
async fn an_inconsistent_breakdown_is_stored_as_unclassified() {
    let gateway = gateway().await.expect("a gateway");
    let mut inconsistent = record(ANTIGRAVITY, "antigravity-1").expect("a reference record");
    inconsistent["token_breakdown"]["quality"] = json!("inconsistent");
    inconsistent["token_breakdown"]["unclassified_tokens"] = json!(12345);

    post_batch(
        &gateway.client,
        &gateway.source_id,
        &json!({ "records": [inconsistent] }),
    )
    .await;

    let events = events(&gateway.client).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].token_quality.as_deref(), Some("inconsistent"));
    assert_eq!(events[0].unclassified_tokens, 12345);
    assert_eq!(
        summary(&gateway.client).await.metrics.unclassified_tokens,
        12345
    );
}

#[tokio::test]
async fn an_unreadable_record_is_kept_without_its_secrets() {
    let gateway = gateway().await.expect("a gateway");
    let mut malformed = record(CLAUDE, "0b7e44d2").expect("a reference record");
    malformed["timestamp"] = json!("yesterday");
    malformed["auth_type"] = json!("apikey");
    malformed["source"] = json!(API_KEY);
    malformed["api_key"] = json!(API_KEY);

    let ingested = post_batch(
        &gateway.client,
        &gateway.source_id,
        &json!({ "records": [malformed, record(GROK, "xai-1").expect("a reference record")] }),
    )
    .await;
    assert_eq!(
        ingested.accepted, 1,
        "one bad record does not stop the batch"
    );
    assert_eq!(ingested.rejected, 1);

    let body = body_of(&gateway.client, "/api/dead-letters")
        .await
        .expect("a dead letter list");
    assert!(!body.contains(API_KEY), "{body}");
    let dead_letters = serde_json::from_str::<DeadLetterList>(&body)
        .expect("a dead letter list")
        .dead_letters;
    assert_eq!(dead_letters.len(), 1);
    let dead_letter = &dead_letters[0];
    assert_eq!(dead_letter.source_id, gateway.source_id);
    assert!(
        dead_letter.error.contains("yesterday"),
        "{}",
        dead_letter.error
    );
    let payload = serde_json::from_str::<Value>(&dead_letter.payload).expect("the record");
    assert_eq!(payload["timestamp"], json!("yesterday"));
    assert_eq!(payload["auth_index"], json!("0b7e44d2"));
    for secret in ["api_key", "source", "response_headers"] {
        assert!(payload.get(secret).is_none(), "{secret} was kept");
    }
}

#[tokio::test]
async fn a_merged_account_hands_its_events_and_its_credential_to_the_target() {
    let gateway = gateway().await.expect("a gateway");
    post_batch(
        &gateway.client,
        &gateway.source_id,
        &batch().expect("the reference records"),
    )
    .await;
    let listed = accounts(&gateway.client).await;
    let account_of = |provider: &str| {
        listed
            .iter()
            .find(|account| account.provider == provider)
            .expect("the account")
            .account_id
            .clone()
    };
    let (target, merged) = (account_of("antigravity"), account_of("claude"));

    let merge = |from: &str, into: &str| {
        gateway
            .client
            .post(format!("/api/accounts/{from}/merge"))
            .body_json(&json!({ "into_account_id": into }))
            .send()
    };
    merge(&merged, &merged)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    merge(&merged, "00000000000")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    let merged_response = merge(&merged, &target).await;
    merged_response.assert_status_is_ok();
    let absorbed = merged_response
        .json()
        .await
        .value()
        .deserialize::<ListedAccount>();
    assert_eq!(absorbed.account_id, target);
    assert_eq!(absorbed.merged_into_account_id, None);
    assert_eq!(absorbed.event_count, 2);
    merge(&account_of("xai"), &merged)
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    let remaining = accounts(&gateway.client).await;
    assert_eq!(remaining.len(), 2, "a merged account is not listed");
    assert!(remaining.iter().all(|account| account.account_id != merged));
    let breakdown = gateway
        .client
        .get("/api/analytics/breakdown?group_by=account")
        .send()
        .await;
    breakdown.assert_status_is_ok();
    let breakdown = breakdown.json().await.value().deserialize::<Breakdown>();
    assert!(breakdown.rows.iter().all(|row| row.key != merged));
    let absorbed = breakdown
        .rows
        .iter()
        .find(|row| row.key == target)
        .expect("the target's row");
    assert_eq!(absorbed.metrics.requests, 2);

    let mut later = record(CLAUDE, "claude-1").expect("a reference record");
    later["timestamp"] = json!("2026-01-15T11:00:00+02:00");
    let ingested = post_batch(
        &gateway.client,
        &gateway.source_id,
        &json!({ "records": [later] }),
    )
    .await;
    assert_eq!(ingested.accepted, 1);
    let charged = events(&gateway.client)
        .await
        .into_iter()
        .filter(|event| event.provider == "claude")
        .map(|event| event.account.account_id)
        .collect::<Vec<_>>();
    assert_eq!(charged, vec![target.clone(), target.clone()]);
    assert_eq!(accounts(&gateway.client).await.len(), 2);
}

#[tokio::test]
async fn a_replayed_batch_stores_nothing_twice() {
    let gateway = gateway().await.expect("a gateway");
    let batch = batch().expect("the reference records");

    post_batch(&gateway.client, &gateway.source_id, &batch).await;
    let replayed = post_batch(&gateway.client, &gateway.source_id, &batch).await;
    assert_eq!(replayed.accepted, 0);
    assert_eq!(replayed.duplicates, 3);
    assert_eq!(replayed.rejected, 0);

    assert_eq!(events(&gateway.client).await.len(), 3);
    assert_eq!(accounts(&gateway.client).await.len(), 3);
}

#[tokio::test]
async fn a_series_groups_a_day_under_the_source_that_reported_it() {
    let gateway = gateway().await.expect("a gateway");
    let batch = batch().expect("the reference records");

    post_batch(&gateway.client, &gateway.source_id, &batch).await;

    let series = gateway
        .client
        .get(
            "/api/analytics/series?bucket=day&group_by=source\
             &from=2026-01-15T00:00:00Z&to=2026-01-16T00:00:00Z",
        )
        .send()
        .await;
    series.assert_status_is_ok();
    let series = series.json().await.value().deserialize::<Series>();

    assert_eq!(series.buckets.len(), 1);
    let bucket = &series.buckets[0];
    assert_eq!(bucket.start, "2026-01-15T00:00:00+00:00");
    assert_eq!(bucket.key, gateway.source_id);
    assert_eq!(bucket.metrics.requests, 3);
    assert_eq!(bucket.metrics.total_tokens, 12345);
}

#[tokio::test]
async fn usage_is_read_only_with_the_token() {
    let gateway = gateway().await.expect("a gateway");

    let refused = gateway.anonymous.get("/api/accounts").send().await;
    refused.assert_status(StatusCode::UNAUTHORIZED);
    let refused = gateway
        .anonymous
        .get("/api/accounts")
        .header(header::AUTHORIZATION, "Bearer not the token")
        .send()
        .await;
    refused.assert_status(StatusCode::UNAUTHORIZED);

    let admitted = gateway
        .anonymous
        .get("/api/accounts")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .send()
        .await;
    admitted.assert_status_is_ok();
}

#[tokio::test]
async fn a_session_cookie_stands_in_for_the_token() {
    let gateway = gateway().await.expect("a gateway");

    let wrong = gateway
        .anonymous
        .post("/api/session")
        .body_json(&json!({ "token": "not the token" }))
        .send()
        .await;
    wrong.assert_status(StatusCode::UNAUTHORIZED);

    let opened = gateway
        .anonymous
        .post("/api/session")
        .body_json(&json!({ "token": TOKEN }))
        .send()
        .await;
    opened.assert_status(StatusCode::NO_CONTENT);
    let set_cookie = opened
        .0
        .headers()
        .get(header::SET_COOKIE)
        .expect("a session cookie")
        .to_str()
        .expect("a readable cookie");
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("a name and value")
        .to_owned();

    let admitted = gateway
        .anonymous
        .get("/api/accounts")
        .header(header::COOKIE, cookie)
        .send()
        .await;
    admitted.assert_status_is_ok();
}

#[tokio::test]
async fn a_reconnecting_stream_replays_what_it_missed_in_order() {
    let gateway = gateway().await.expect("a gateway");
    post_batch(
        &gateway.client,
        &gateway.source_id,
        &batch().expect("the reference records"),
    )
    .await;
    let mut ingested = events(&gateway.client)
        .await
        .into_iter()
        .map(|event| event.event_id)
        .collect::<Vec<_>>();
    ingested.reverse();
    let first = ingested.remove(0);

    let stream = gateway
        .client
        .get("/api/usage/stream")
        .header("Last-Event-ID", &first)
        .send()
        .await;
    stream.assert_status_is_ok();
    let mut body = stream.0.into_body().into_bytes_stream();
    let mut text = String::new();
    // The stream stays open for live events, so read only until the replay has arrived.
    tokio::time::timeout(Duration::from_secs(5), async {
        while text.matches("\n\n").count() < ingested.len() {
            let chunk = body
                .next()
                .await
                .expect("more of the stream")
                .expect("bytes");
            text.push_str(std::str::from_utf8(&chunk).expect("text"));
        }
    })
    .await
    .expect("the replay within five seconds");

    let messages = text
        .split("\n\n")
        .filter(|message| !message.is_empty())
        .map(|message| {
            let field = |name: &str| {
                message
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .expect("the field")
                    .to_owned()
            };
            let data = serde_json::from_str::<Value>(&field("data: ")).expect("a usage event");
            assert_eq!(field("event: "), "usage");
            assert_eq!(data["event_id"], json!(field("id: ")));

            field("id: ")
        })
        .collect::<Vec<_>>();
    assert_eq!(messages, ingested);
}
