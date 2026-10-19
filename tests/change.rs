use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use metered_usage::account::Account;
use metered_usage::app::AppState;
use metered_usage::change::{Change, PAGE, RETENTION, Subject};
use metered_usage::config::Config;
use metered_usage::database::{Database, DatabaseError};
use metered_usage::http::auth::Token;
use metered_usage::id::Id;
use metered_usage::source::Source;
use poem::http::{StatusCode, header};
use poem::listener::{Acceptor, Listener, TcpListener};
use poem::test::TestClient;
use poem::web::{Data, Json};
use poem::{EndpointExt, IntoResponse, Request, Response, Route, Server, get, handler, post};
use serde_json::{Value, json};
use tokio::time::Instant;

const TEST_ACCESS_TOKEN: &str = "test-access-token-not-a-secret";
const TEST_MANAGEMENT_KEY: &str = "test-management-key-not-a-secret";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// What the fake CLIProxy and its provider report.
struct Reported {
    /// The share of the five-hour window used.
    used: f64,
    /// When the credential's one cooldown ends. CLIProxy counts its `remaining_seconds`
    /// down on every listing.
    retry_at: &'static str,
    listings: i64,
    /// When the five-hour window resets.
    resets_at: &'static str,
}

/// A CLIProxy that lists one Claude credential with one cooldown, and whose provider
/// reports what the test last set.
#[derive(Clone)]
struct Upstream(Arc<Mutex<Reported>>);

impl Upstream {
    fn new(used: f64) -> Self {
        Self(Arc::new(Mutex::new(Reported {
            used,
            retry_at: "2099-01-01T18:00:00Z",
            listings: 0,
            resets_at: "2099-01-01T00:00:00Z",
        })))
    }

    fn with<T>(&self, read: impl FnOnce(&mut Reported) -> T) -> T {
        match self.0.lock() {
            Ok(mut reported) => read(&mut reported),
            Err(poisoned) => read(&mut poisoned.into_inner()),
        }
    }
}

fn authorized(request: &Request) -> bool {
    request.header(header::AUTHORIZATION) == Some(&format!("Bearer {TEST_MANAGEMENT_KEY}"))
}

#[handler]
fn auth_files(request: &Request, Data(upstream): Data<&Upstream>) -> Response {
    if !authorized(request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (retry_at, listings) = upstream.with(|reported| {
        reported.listings += 1;
        (reported.retry_at, reported.listings)
    });
    Json(json!({ "files": [{
        "id": "claude.json", "auth_index": "a1", "name": "claude.json",
        "type": "claude", "provider": "claude", "email": "ada@example.com",
        "account_type": "oauth", "status": "active", "disabled": false,
        "unavailable": false,
        "cooldowns": [{
            "scope": "model", "model_key": "claude-opus-4", "reason": "quota",
            "retry_at": retry_at, "remaining_seconds": 3600 - listings,
        }],
    }] }))
    .into_response()
}

#[handler]
fn api_call(
    request: &Request,
    Data(upstream): Data<&Upstream>,
    Json(call): Json<Value>,
) -> Response {
    if !authorized(request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (status, body) = if call["url"] == USAGE_URL {
        (
            200,
            json!({ "five_hour": {
                "utilization": upstream.with(|reported| reported.used) * 100.0,
                "resets_at": upstream.with(|reported| reported.resets_at),
            } }),
        )
    } else {
        (401, json!({ "error": "unauthorized" }))
    };
    Json(json!({ "status_code": status, "header": {}, "body": body.to_string() })).into_response()
}

async fn service(
    upstream: &Upstream,
) -> Result<(TestClient<impl poem::Endpoint>, Arc<AppState>), Box<dyn std::error::Error>> {
    let acceptor = TcpListener::bind("127.0.0.1:0").into_acceptor().await?;
    let address = acceptor
        .local_addr()
        .first()
        .and_then(|address| address.as_socket_addr().copied())
        .ok_or("the listener has no socket address")?;
    let app = Route::new()
        .at("/v0/management/auth-files", get(auth_files))
        .at("/v0/management/api-call", post(api_call))
        .data(upstream.clone());
    tokio::spawn(Server::new_with_acceptor(acceptor).run(app));

    let config = Config::parse(
        &format!(
            "[[source]]\nkey = \"work\"\nkind = \"cliproxy\"\nbase_url = \"http://{address}\"\nkey_env = \"CLIPROXY_KEY\"\n"
        ),
        |variable| (variable == "CLIPROXY_KEY").then(|| TEST_MANAGEMENT_KEY.to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let token = Token::try_from(TEST_ACCESS_TOKEN.to_owned())?;
    let state = Arc::new(AppState::new(database, token.into(), config)?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TEST_ACCESS_TOKEN}"));

    Ok((client, state))
}

async fn changes(client: &TestClient<impl poem::Endpoint>, query: &str) -> (StatusCode, Value) {
    let response = client.get(format!("/api/events?{query}")).send().await;
    let status = response.0.status();
    (status, response.json().await.value().deserialize::<Value>())
}

/// The cursor a reader that has read nothing yet starts from.
async fn latest(client: &TestClient<impl poem::Endpoint>) -> i64 {
    let (status, answer) = changes(client, "").await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["events"], json!([]));
    let cursor = answer["cursor"].as_i64();
    assert!(cursor.is_some(), "{answer}");
    cursor.unwrap_or_default()
}

async fn send(client: &TestClient<impl poem::Endpoint>, path: &str, body: &Value) -> Value {
    let response = client.post(path).body_json(body).send().await;
    response.assert_status_is_ok();
    response.json().await.value().deserialize::<Value>()
}

async fn hard_refresh(client: &TestClient<impl poem::Endpoint>) -> Value {
    send(client, "/api/quota/refresh", &json!({})).await
}

/// Each change as its kind and what it names.
fn named(answer: &Value) -> Vec<String> {
    answer["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|change| {
            let names = ["window_key", "model"]
                .into_iter()
                .filter_map(|field| change[field].as_str())
                .collect::<Vec<_>>();
            [change["kind"].as_str().unwrap_or("?")]
                .into_iter()
                .chain(names)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Empty when absent, so the request that names it fails.
fn account_id(quota: &Value) -> String {
    quota["accounts"][0]["account"]["account_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn a_hard_refresh_reaches_a_waiting_long_poll_within_a_second() {
    let upstream = Upstream::new(0.37);
    let (client, state) = service(&upstream).await.expect("a service");
    send(&client, "/api/quota/sync", &json!({})).await;
    let cursor = latest(&client).await;

    let waiting = async {
        let answer = changes(&client, &format!("cursor={cursor}&timeout=30")).await;
        (answer, Instant::now())
    };
    let refreshing = async {
        // Refresh only once the poll is subscribed, so its answer comes from the wakeup.
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.database.waiting_for_changes() == 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(state.database.waiting_for_changes(), 1);
        let started = Instant::now();
        hard_refresh(&client).await;
        started
    };
    let (((status, answer), answered_at), started) = tokio::join!(waiting, refreshing);

    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        named(&answer),
        ["account.changed", "quota.updated five-hour"]
    );
    let elapsed = answered_at.saturating_duration_since(started);
    assert!(
        elapsed < Duration::from_secs(1),
        "answered {elapsed:?} after the refresh"
    );
    let cursors = answer["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| change["cursor"].as_i64())
        .collect::<Vec<_>>();
    assert!(cursors.iter().all(|change| *change > cursor), "{answer}");
    assert_eq!(answer["cursor"].as_i64(), cursors.last().copied());
}

#[tokio::test]
async fn a_cursor_resumes_after_the_last_change_it_read() {
    let upstream = Upstream::new(0.37);
    let (client, _state) = service(&upstream).await.expect("a service");
    let account = account_id(&hard_refresh(&client).await);
    let seen = latest(&client).await;

    let (_, nothing) = changes(&client, &format!("cursor={seen}&timeout=0")).await;
    assert_eq!(nothing["events"], json!([]));
    assert_eq!(nothing["cursor"], seen);

    // The same report changes no window, only when the account was last refreshed. A reset
    // a provider gives as seconds from now lands a second or two apart on each answer.
    upstream.with(|reported| reported.resets_at = "2099-01-01T00:00:02Z");
    hard_refresh(&client).await;
    upstream.with(|reported| reported.used = 0.5);
    hard_refresh(&client).await;
    let response = client
        .patch(format!("/api/accounts/{account}"))
        .body_json(&json!({ "display_name": "Ada's Max" }))
        .send()
        .await;
    response.assert_status_is_ok();
    send(
        &client,
        "/api/prices",
        &json!({
            "model": "claude-opus-4",
            "input_usd_per_mtok": 15.0,
            "cached_input_usd_per_mtok": 1.5,
            "cache_write_usd_per_mtok": 18.75,
            "output_usd_per_mtok": 75.0,
        }),
    )
    .await;

    let (status, resumed) = changes(&client, &format!("cursor={seen}")).await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    assert_eq!(
        named(&resumed),
        [
            "account.changed",
            "account.changed",
            "quota.updated five-hour",
            "account.changed",
            "price.changed claude-opus-4",
        ]
    );
    let next = resumed["cursor"].as_i64().expect("a cursor");
    let (_, after) = changes(&client, &format!("cursor={next}&timeout=0")).await;
    assert_eq!(after["events"], json!([]));

    let (_, quota_only) = changes(
        &client,
        &format!("cursor={seen}&kinds=quota.updated,reset.updated"),
    )
    .await;
    assert_eq!(named(&quota_only), ["quota.updated five-hour"]);
    assert_eq!(
        quota_only["cursor"], next,
        "the cursor passes over the kinds left out"
    );

    let (status, refused) = changes(&client, "cursor=0&kinds=usage.ingested").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}

#[tokio::test]
async fn a_cursor_past_the_retention_answers_gone_with_the_oldest_cursor() {
    let upstream = Upstream::new(0.37);
    let (client, state) = service(&upstream).await.expect("a service");
    let account = account_id(&hard_refresh(&client).await);
    let pruned_through = latest(&client).await;
    assert!(pruned_through > 0);

    let later = Timestamp::now() + RETENTION + SignedDuration::from_hours(1);
    let pruned = Change::prune(&state.database, later).await.expect("pruned");
    assert!(pruned > 0);

    for stale in [0, pruned_through - 1, pruned_through + 1] {
        let (status, gone) = changes(&client, &format!("cursor={stale}&timeout=0")).await;
        assert_eq!(status, StatusCode::GONE, "cursor {stale}: {gone}");
        assert_eq!(gone["oldest_cursor"], pruned_through);
        assert_eq!(gone["latest_cursor"], pruned_through);
    }

    let (status, current) = changes(&client, &format!("cursor={pruned_through}&timeout=0")).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["events"], json!([]));

    // A cursor is never handed out twice, even after pruning emptied the table.
    let response = client
        .patch(format!("/api/accounts/{account}"))
        .body_json(&json!({ "display_name": "renamed" }))
        .send()
        .await;
    response.assert_status_is_ok();
    let (_, renamed) = changes(&client, &format!("cursor={pruned_through}")).await;
    assert_eq!(named(&renamed), ["account.changed"]);
    assert_eq!(renamed["events"][0]["cursor"], pruned_through + 1);
}

#[tokio::test]
async fn an_unchanged_list_answers_not_modified() {
    let upstream = Upstream::new(0.37);
    let (client, _state) = service(&upstream).await.expect("a service");
    let account = account_id(&hard_refresh(&client).await);
    let tag = |response: &poem::test::TestResponse| {
        response
            .0
            .headers()
            .get(header::ETAG)
            .and_then(|tag| tag.to_str().ok())
            .expect("an ETag")
            .to_owned()
    };

    let mut tags = Vec::new();
    for path in ["/api/quota", "/api/prices", "/api/accounts"] {
        let first = client.get(path).send().await;
        first.assert_status_is_ok();
        let current = tag(&first);

        for if_none_match in [current.clone(), format!("W/{current}"), "*".to_owned()] {
            let again = client
                .get(path)
                .header(header::IF_NONE_MATCH, &if_none_match)
                .send()
                .await;
            again.assert_status(StatusCode::NOT_MODIFIED);
            assert_eq!(tag(&again), current, "{path}");
            again.assert_text("").await;
        }
        let other = client
            .get(path)
            .header(header::IF_NONE_MATCH, "\"another\"")
            .send()
            .await;
        other.assert_status_is_ok();
        tags.push((path, current));
    }

    let response = client
        .patch(format!("/api/accounts/{account}"))
        .body_json(&json!({ "display_name": "renamed" }))
        .send()
        .await;
    response.assert_status_is_ok();
    send(
        &client,
        "/api/prices",
        &json!({
            "model": "claude-opus-4",
            "input_usd_per_mtok": 15.0,
            "cached_input_usd_per_mtok": 1.5,
            "cache_write_usd_per_mtok": 18.75,
            "output_usd_per_mtok": 75.0,
        }),
    )
    .await;

    for (path, previous) in tags {
        let changed = client
            .get(path)
            .header(header::IF_NONE_MATCH, &previous)
            .send()
            .await;
        changed.assert_status_is_ok();
        assert_ne!(tag(&changed), previous, "{path}");
    }
}

#[tokio::test]
async fn a_soft_sync_records_only_what_moved() {
    let upstream = Upstream::new(0.37);
    let (client, _state) = service(&upstream).await.expect("a service");
    send(&client, "/api/quota/sync", &json!({})).await;
    let seen = latest(&client).await;

    // CLIProxy counts the cooldown down on every listing; when it ends has not moved.
    send(&client, "/api/quota/sync", &json!({})).await;
    let (_, unchanged) = changes(&client, &format!("cursor={seen}&timeout=0")).await;
    assert_eq!(named(&unchanged), Vec::<String>::new());

    upstream.with(|reported| reported.retry_at = "2099-01-01T19:00:00Z");
    send(&client, "/api/quota/sync", &json!({})).await;
    let (_, moved) = changes(&client, &format!("cursor={seen}&timeout=0")).await;
    assert_eq!(named(&moved), ["account.changed"]);
}

#[tokio::test]
async fn a_partial_prune_keeps_the_changes_inside_the_retention() {
    let upstream = Upstream::new(0.37);
    let (client, state) = service(&upstream).await.expect("a service");
    record_prices(&state, &["first", "second"])
        .await
        .expect("recorded");
    let horizon = latest(&client).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let between = Timestamp::now();
    tokio::time::sleep(Duration::from_millis(5)).await;
    record_prices(&state, &["third"]).await.expect("recorded");

    let pruned = Change::prune(&state.database, between + RETENTION)
        .await
        .expect("pruned");
    assert_eq!(pruned, 2);

    let (status, kept) = changes(&client, &format!("cursor={horizon}&timeout=0")).await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(named(&kept), ["price.changed third"]);
    let (status, gone) = changes(&client, &format!("cursor={}&timeout=0", horizon - 1)).await;
    assert_eq!(status, StatusCode::GONE, "{gone}");
    assert_eq!(gone["oldest_cursor"], horizon);
    assert_eq!(gone["latest_cursor"], horizon + 1);
}

#[tokio::test]
async fn a_full_page_answers_the_cursor_of_its_last_change() {
    let upstream = Upstream::new(0.37);
    let (client, state) = service(&upstream).await.expect("a service");
    let start = latest(&client).await;
    let models = (0..=PAGE).map(|n| format!("model-{n}")).collect::<Vec<_>>();
    let models = models.iter().map(String::as_str).collect::<Vec<_>>();
    record_prices(&state, &models).await.expect("recorded");
    let mut write = state.database.write().await.expect("a write");
    Change::record(&mut write, &Subject::Account(Id::<Account>::from_raw(1)))
        .await
        .expect("recorded");
    write.commit().await.expect("committed");
    let last = start + PAGE + 2;

    for kinds in ["", "&kinds=price.changed"] {
        let (_, full) = changes(&client, &format!("cursor={start}{kinds}")).await;
        let events = full["events"].as_array().map_or(0, Vec::len);
        assert_eq!(i64::try_from(events), Ok(PAGE), "{kinds}");
        assert_eq!(full["cursor"], start + PAGE, "{kinds}");

        let (_, rest) = changes(
            &client,
            &format!("cursor={}{kinds}&timeout=0", start + PAGE),
        )
        .await;
        let expected = if kinds.is_empty() {
            vec![
                format!("price.changed model-{PAGE}"),
                "account.changed".to_owned(),
            ]
        } else {
            vec![format!("price.changed model-{PAGE}")]
        };
        assert_eq!(named(&rest), expected, "{kinds}");
        assert_eq!(rest["cursor"], last, "{kinds}");
    }
}

/// Records a `price.changed` for each model in one write.
async fn record_prices(state: &AppState, models: &[&str]) -> Result<(), DatabaseError> {
    let mut write = state.database.write().await?;
    for model in models {
        Change::record(
            &mut write,
            &Subject::Price {
                model: (*model).to_owned(),
            },
        )
        .await?;
    }
    write.commit().await
}
