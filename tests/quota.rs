use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::database::codec::StoredTimestamp;
use metered_usage::http::auth::Token;
use metered_usage::source::Source;
use poem::http::{StatusCode, header};
use poem::listener::{Acceptor, Listener, TcpListener};
use poem::test::TestClient;
use poem::web::{Data, Json, Query};
use poem::{EndpointExt, IntoResponse, Request, Response, Route, Server, get, handler, post};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;

const TOKEN: &str = "correct horse battery staple";
const MANAGEMENT_KEY: &str = "mgmt-key-7f3a9c1e5b";
const DCA_TOKEN: &str = "dca:META-DCA-SECRET-4411";
const META_LLM_KEY: &str = "meta-llm-key-SECRET-9921";
const API_KEY_LABEL: &str = "sk-proj-abcdefghijklWXYZ";

/// Every `api-call` body the fake CLIProxy received.
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<Value>>>);

impl Calls {
    fn all(&self) -> Vec<Value> {
        self.0.lock().map(|calls| calls.clone()).unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Download {
    name: String,
}

fn authorized(request: &Request) -> bool {
    request.header(header::AUTHORIZATION) == Some(&format!("Bearer {MANAGEMENT_KEY}"))
}

#[handler]
fn auth_files(request: &Request) -> Response {
    if !authorized(request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({
        "observed_at": "2026-09-23T10:00:00Z",
        "files": [
            {
                "id": "claude-ada.json", "auth_index": "a1", "name": "claude-ada.json",
                "type": "claude", "provider": "claude", "label": "", "email": "ada@example.com",
                "account_type": "oauth", "status": "active", "status_message": "",
                "disabled": false, "unavailable": true, "runtime_only": false,
                "next_retry_after": "2026-09-23T18:00:00Z",
                "cooldowns": [{
                    "scope": "model", "model_key": "claude-opus-4", "reason": "quota",
                    "retry_at": "2026-09-23T18:00:00Z", "remaining_seconds": 3600,
                    "http_status": 429,
                }],
                "quota": { "signals": {} },
            },
            {
                "id": "codex-bo.json", "auth_index": "a2", "name": "codex-bo.json",
                "type": "codex", "provider": "codex", "label": "", "email": "bo@example.com",
                "status": "active", "disabled": false, "unavailable": false,
                "id_token": { "chatgpt_account_id": "acct-77", "plan_type": "Plus" },
                "cooldowns": [],
            },
            {
                "id": "devin.json", "auth_index": "a3", "name": "devin.json",
                "type": "devin", "provider": "devin", "label": "devin-login",
                "status": "active", "disabled": false, "unavailable": false, "cooldowns": [],
            },
            {
                "id": "kimi.json", "auth_index": "a4", "name": "kimi.json",
                "type": "kimi", "provider": "kimi", "label": "", "email": "kim@example.com",
                "status": "active", "disabled": false, "unavailable": false, "cooldowns": null,
            },
            {
                "id": "meta.json", "auth_index": "a5", "name": "meta.json",
                "type": "meta", "provider": "meta", "label": "", "email": "mo@example.com",
                "status": "active", "disabled": false, "unavailable": false,
                "runtime_only": false, "cooldowns": [],
            },
            {
                "id": "compat", "auth_index": "a6", "name": "compat",
                "type": "openai-compatibility", "provider": "openai-compatibility",
                "label": API_KEY_LABEL, "account_type": "api_key", "account": API_KEY_LABEL,
                "status": "active", "disabled": false, "unavailable": false, "cooldowns": [],
            },
            { "name": "no-index.json", "type": "claude" },
        ],
    }))
    .into_response()
}

#[handler]
fn download(request: &Request, Query(query): Query<Download>) -> Response {
    if !authorized(request) || query.name != "meta.json" {
        return StatusCode::NOT_FOUND.into_response();
    }
    json!({ "type": "meta", "dca_token": DCA_TOKEN, "api_key": META_LLM_KEY })
        .to_string()
        .into_response()
}

#[handler]
fn api_call(request: &Request, Data(calls): Data<&Calls>, Json(call): Json<Value>) -> Response {
    if !authorized(request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Ok(mut received) = calls.0.lock() {
        received.push(call.clone());
    }
    let url = call["url"].as_str().unwrap_or_default();
    let (status, body) = match url {
        "https://api.anthropic.com/api/oauth/usage" => (
            200,
            json!({
                "five_hour": { "utilization": 37.0, "resets_at": "2026-09-23T15:00:00.123456+00:00" },
                "seven_day": { "utilization": 62, "resets_at": "2026-09-28T09:00:00+00:00" },
                "seven_day_oauth_apps": null,
                "seven_day_opus": null,
                "extra_usage": { "is_enabled": false, "monthly_limit": null, "used_credits": null, "utilization": null },
            }),
        ),
        "https://api.anthropic.com/api/oauth/profile" => (
            200,
            json!({
                "account": { "email": "ada@example.com", "has_claude_max": true, "has_claude_pro": false },
                "organization": { "organization_type": "claude_max", "subscription_status": "active" },
            }),
        ),
        "https://chatgpt.com/backend-api/wham/usage" => (
            200,
            json!({
                "plan_type": "plus",
                "rate_limit": {
                    "allowed": true,
                    "limit_reached": false,
                    "primary_window": { "used_percent": 24, "limit_window_seconds": 18000, "reset_after_seconds": 7200, "reset_at": 1_790_000_000 },
                    "secondary_window": { "used_percent": 51, "limit_window_seconds": 604_800, "reset_after_seconds": 300_000, "reset_at": 1_790_300_000 },
                },
                "code_review_rate_limit": null,
                "additional_rate_limits": null,
            }),
        ),
        "https://server.codeium.com/exa.seat_management_pb.SeatManagementService/GetUserStatus" => {
            (
                200,
                json!({
                    "userStatus": { "planStatus": {
                        "planInfo": { "planName": "Pro" },
                        "dailyQuotaRemainingPercent": 80,
                        "dailyQuotaResetAtUnix": "1790000000",
                        "weeklyQuotaRemainingPercent": 35.5,
                        "weeklyQuotaResetAtUnix": 1_790_400_000,
                    } },
                }),
            )
        }
        "https://api.kimi.com/coding/v1/usages" => (503, json!({ "error": "upstream overloaded" })),
        "https://api.meta.ai/muse-code/key"
            if call["header"]["Authorization"] == format!("Bearer {DCA_TOKEN}") =>
        {
            (
                200,
                json!({
                    "api_key": META_LLM_KEY,
                    "subs_tier_name": "Muse Plus",
                    "is_subs_active": true,
                    "subs_usage": {
                        "window": { "used_percent": 12.5, "resets_at": 1_790_000_000, "window_duration_mins": 300 },
                        "weekly": { "used_percent": 40, "resets_at": 1_790_500_000 },
                    },
                }),
            )
        }
        _ => (401, json!({ "error": "unauthorized" })),
    };
    Json(json!({ "status_code": status, "header": {}, "body": body.to_string() })).into_response()
}

/// A CLIProxy management API on a random local port.
async fn cliproxy(calls: Calls) -> Result<String, Box<dyn std::error::Error>> {
    let acceptor = TcpListener::bind("127.0.0.1:0").into_acceptor().await?;
    let address = acceptor
        .local_addr()
        .first()
        .and_then(|address| address.as_socket_addr().copied())
        .ok_or("the listener has no socket address")?;
    let app = Route::new()
        .at("/v0/management/auth-files", get(auth_files))
        .at("/v0/management/auth-files/download", get(download))
        .at("/v0/management/api-call", post(api_call))
        .data(calls);
    tokio::spawn(Server::new_with_acceptor(acceptor).run(app));

    Ok(format!("http://{address}"))
}

async fn service(
    calls: &Calls,
) -> Result<(TestClient<impl poem::Endpoint>, Arc<AppState>), Box<dyn std::error::Error>> {
    let base_url = cliproxy(calls.clone()).await?;
    let config = Config::parse(
        &format!(
            "[[source]]\nkey = \"work\"\nkind = \"cliproxy\"\nbase_url = \"{base_url}\"\nkey_env = \"CLIPROXY_KEY\"\n"
        ),
        |variable| (variable == "CLIPROXY_KEY").then(|| MANAGEMENT_KEY.to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let token = Token::try_from(TOKEN.to_owned())?;
    let state = Arc::new(AppState::new(database, token, config)?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));

    Ok((client, state))
}

/// Null when absent, so the assertion that reads it fails.
fn by_provider<'a>(accounts: &'a Value, provider: &str) -> &'a Value {
    accounts["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|account| account["account"]["provider"] == provider)
        .unwrap_or(&Value::Null)
}

/// Null when absent, so the assertion that reads it fails.
fn window<'a>(account: &'a Value, key: &str) -> &'a Value {
    account["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|window| window["window_key"] == key)
        .unwrap_or(&Value::Null)
}

fn fraction(window: &Value) -> f64 {
    window["used_fraction"].as_f64().unwrap_or(f64::NAN)
}

/// Every stored value of every table, as text.
async fn stored_text(state: &AppState) -> Result<String, sqlx::Error> {
    let pool = &state.database.pool;
    let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(pool)
        .await?;
    let mut text = String::new();
    for table in tables {
        let table: String = table.get(0);
        let columns = sqlx::query("SELECT name FROM pragma_table_info(?)")
            .bind(&table)
            .fetch_all(pool)
            .await?;
        for column in columns {
            let column: String = column.get(0);
            let values: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT group_concat(quote(\"{column}\"), char(10)) FROM \"{table}\""
            )))
            .fetch_one(pool)
            .await?;
            text.push_str(&values.unwrap_or_default());
            text.push('\n');
        }
    }

    Ok(text)
}

#[tokio::test]
async fn soft_sync_registers_every_credential_without_reaching_a_provider() {
    let calls = Calls::default();
    let (client, _state) = service(&calls).await.expect("a service");

    let response = client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await;
    response.assert_status_is_ok();
    let synced = response.json().await.value().deserialize::<Value>();

    assert!(calls.all().is_empty(), "a soft sync sent {:?}", calls.all());
    assert_eq!(synced["accounts"].as_array().expect("accounts").len(), 6);

    let claude = by_provider(&synced, "claude");
    assert_eq!(claude["account"]["label"], "ada@example.com");
    assert_eq!(claude["unavailable"], true);
    assert_eq!(claude["next_retry_after"], "2026-09-23T18:00:00Z");
    assert_eq!(claude["cooldowns"][0]["model_key"], "claude-opus-4");
    assert_eq!(claude["cooldowns"][0]["reason"], "quota");
    assert_eq!(claude["cooldowns"][0]["remaining_seconds"], 3600);
    assert!(claude["windows"].as_array().expect("windows").is_empty());

    assert_eq!(by_provider(&synced, "codex")["plan"], "plus");

    let kimi = by_provider(&synced, "kimi");
    assert_eq!(kimi["cooldowns"], json!([]));
    assert_eq!(kimi["soft_observed_at"], "2026-09-23T10:00:00Z");

    let compatible = by_provider(&synced, "openai-compatibility");
    assert_eq!(compatible["account"]["auth_kind"], "api_key");
    assert_eq!(compatible["account"]["label"], "…WXYZ");
    assert_eq!(compatible["hard_refreshable"], false);
    assert_eq!(by_provider(&synced, "meta")["hard_refreshable"], true);

    let accounts = client.get("/api/accounts").send().await;
    accounts.assert_status_is_ok();
    let accounts = accounts.json().await.value().deserialize::<Value>();
    assert_eq!(accounts["accounts"].as_array().expect("accounts").len(), 6);
}

#[tokio::test]
async fn hard_refresh_stores_used_fractions_and_isolates_a_failing_credential() {
    let calls = Calls::default();
    let (client, _state) = service(&calls).await.expect("a service");

    let response = client
        .post("/api/quota/refresh")
        .body_json(&json!({}))
        .send()
        .await;
    response.assert_status_is_ok();
    let refreshed = response.json().await.value().deserialize::<Value>();

    assert_eq!(refreshed["refreshed"], 4);
    assert_eq!(refreshed["failed"], 1);

    let claude = by_provider(&refreshed, "claude");
    let five_hour = window(claude, "five-hour");
    assert!((fraction(five_hour) - 0.37).abs() < 1e-9);
    assert_eq!(five_hour["window_seconds"], 18_000);
    assert_eq!(five_hour["resets_at"], "2026-09-23T15:00:00.123456Z");
    assert!((fraction(window(claude, "seven-day")) - 0.62).abs() < 1e-9);
    assert_eq!(claude["plan"], "max");
    assert!(claude.get("hard_refresh_error").is_none());
    // The gateway's scheduling decision stays apart from the provider's reset.
    assert_eq!(claude["next_retry_after"], "2026-09-23T18:00:00Z");

    let codex = by_provider(&refreshed, "codex");
    assert!((fraction(window(codex, "five-hour")) - 0.24).abs() < 1e-9);
    let weekly = window(codex, "weekly");
    assert!((fraction(weekly) - 0.51).abs() < 1e-9);
    assert_eq!(weekly["window_seconds"], 604_800);
    assert_eq!(weekly["resets_at"], "2026-09-25T01:33:20Z");

    let devin = by_provider(&refreshed, "devin");
    assert!((fraction(window(devin, "daily")) - 0.2).abs() < 1e-9);
    assert!((fraction(window(devin, "weekly")) - 0.645).abs() < 1e-9);
    assert_eq!(devin["plan"], "Pro");

    let meta = by_provider(&refreshed, "meta");
    assert!((fraction(window(meta, "window")) - 0.125).abs() < 1e-9);
    assert_eq!(window(meta, "window")["window_seconds"], 18_000);
    assert_eq!(meta["plan"], "Muse Plus");

    let kimi = by_provider(&refreshed, "kimi");
    assert_eq!(kimi["hard_refresh_error"], "the provider answered 503");
    assert!(kimi["windows"].as_array().expect("windows").is_empty());

    let sent = calls.all();
    let claude_usage = sent
        .iter()
        .find(|call| call["url"] == "https://api.anthropic.com/api/oauth/usage")
        .expect("the Claude usage call");
    assert_eq!(claude_usage["authIndex"], "a1");
    assert_eq!(claude_usage["method"], "GET");
    assert_eq!(
        claude_usage["header"],
        json!({
            "Authorization": "Bearer $TOKEN$",
            "Content-Type": "application/json",
            "anthropic-beta": "oauth-2025-04-20",
        })
    );
    let codex_usage = sent
        .iter()
        .find(|call| call["url"] == "https://chatgpt.com/backend-api/wham/usage")
        .expect("the Codex usage call");
    assert_eq!(codex_usage["header"]["Chatgpt-Account-Id"], "acct-77");
    assert!(sent.iter().all(|call| {
        let url = call["url"].as_str().unwrap_or_default();
        !url.ends_with("/consume") && call["authIndex"] != "a6"
    }));

    let listed = client.get("/api/quota").send().await;
    listed.assert_status_is_ok();
    let listed = listed.json().await.value().deserialize::<Value>();
    assert!((fraction(window(by_provider(&listed, "devin"), "weekly")) - 0.645).abs() < 1e-9);
}

#[tokio::test]
async fn a_second_refresh_replaces_every_window() {
    let calls = Calls::default();
    let (client, state) = service(&calls).await.expect("a service");
    client
        .post("/api/quota/refresh")
        .body_json(&json!({}))
        .send()
        .await
        .assert_status_is_ok();
    sqlx::query(
        "INSERT INTO quota_window (account_id, window_key, label, observed_at) \
         SELECT account_id, 'stale', 'Stale', '2026-01-01T00:00:00.000000000Z' FROM account \
         WHERE upstream_key = 'a1'",
    )
    .execute(&state.database.pool)
    .await
    .expect("a stale window");

    let listed = client
        .get("/api/quota")
        .send()
        .await
        .json()
        .await
        .value()
        .deserialize::<Value>();
    let claude = by_provider(&listed, "claude");
    let account_id = claude["account"]["account_id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let response = client
        .post("/api/quota/refresh")
        .body_json(&json!({ "account_id": account_id }))
        .send()
        .await;
    response.assert_status_is_ok();
    let refreshed = response.json().await.value().deserialize::<Value>();

    assert_eq!(refreshed["refreshed"], 1);
    let keys = by_provider(&refreshed, "claude")["windows"]
        .as_array()
        .expect("windows")
        .iter()
        .map(|window| window["window_key"].as_str().expect("a key").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(keys, ["five-hour", "seven-day"]);
}

fn minutes_from(now: Timestamp, minutes: i64) -> String {
    StoredTimestamp::from(now + SignedDuration::from_mins(minutes)).to_string()
}

/// Every window lasts five hours. Minutes are relative to `now`.
async fn insert_windows(pool: &sqlx::SqlitePool, now: Timestamp) -> Result<(), sqlx::Error> {
    for (upstream_key, key, used_fraction, scope, starts_on_use, observed, resets) in [
        ("a1", "opus", 0.2, "model:opus", false, -60, 60),
        ("a1", "sonnet", 0.5, "model:sonnet", false, -100, -30),
        ("a1", "untouched", 0.0, "account", false, -60, 60),
        ("a1", "cowork", 0.3, "unmetered", false, -60, 60),
        ("a2", "five-hour", 0.9, "account", true, -400, -120),
        ("a2", "idle", 0.9, "account", true, -400, -5),
        ("a4", "limit-0", 0.1, "account", false, -60, 60),
    ] {
        sqlx::query(
            "INSERT INTO quota_window (account_id, window_key, label, used_fraction, \
             window_seconds, resets_at, observed_at, scope, starts_on_use) \
             SELECT account_id, ?, ?, ?, 18000, ?, ?, ?, ? FROM account WHERE upstream_key = ?",
        )
        .bind(key)
        .bind(key)
        .bind(used_fraction)
        .bind(minutes_from(now, resets))
        .bind(minutes_from(now, observed))
        .bind(scope)
        .bind(starts_on_use)
        .bind(upstream_key)
        .execute(pool)
        .await?;
    }
    sqlx::query(
        "UPDATE quota_window SET used_value = 10, limit_value = 100, unit = 'requests' \
         WHERE window_key = 'limit-0'",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Opus learns 0.2 per dollar twice, and 0.01 once from a window reset early.
async fn insert_samples(pool: &sqlx::SqlitePool, now: Timestamp) -> Result<(), sqlx::Error> {
    for (upstream_key, key, started, observed, used_fraction) in [
        ("a1", "opus", -240, -60, 0.2),
        ("a1", "opus", -2000, -1800, 0.4),
        ("a1", "opus", -3000, -2800, 0.05),
        ("a1", "sonnet", -330, -100, 0.5),
        ("a2", "five-hour", -420, -400, 0.1),
    ] {
        sqlx::query(
            "INSERT INTO quota_sample (account_id, window_key, started_at, observed_at, \
             used_fraction) SELECT account_id, ?, ?, ?, ? FROM account WHERE upstream_key = ?",
        )
        .bind(key)
        .bind(minutes_from(now, started))
        .bind(minutes_from(now, observed))
        .bind(used_fraction)
        .bind(upstream_key)
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn insert_events(pool: &sqlx::SqlitePool, now: Timestamp) -> Result<(), sqlx::Error> {
    for (index, (upstream_key, model, occurred, failed, cost)) in [
        ("a1", "claude-opus-4", -2900, false, 5.0),
        ("a1", "claude-opus-4", -1900, false, 2.0),
        ("a1", "claude-opus-4", -200, false, 1.0),
        ("a1", "claude-sonnet-4", -200, false, 5.0),
        ("a1", "claude-opus-4", -30, false, 0.5),
        ("a1", "claude-sonnet-4", -30, false, 2.0),
        ("a2", "gpt-5", -410, false, 1.0),
        ("a2", "gpt-5", -100, false, 1.0),
        ("a2", "gpt-5", -10, false, 1.0),
        ("a4", "kimi", -120, false, 0.0),
        ("a4", "kimi", -30, false, 0.0),
        ("a4", "kimi", -20, false, 0.0),
        ("a4", "kimi", -10, true, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO usage_event (source_id, account_id, record_hash, upstream_id, \
             occurred_at, provider, model, endpoint, streamed, status_code, failed, \
             list_cost_usd) \
             SELECT source_id, account_id, ?, ?, ?, provider, ?, '/v1/messages', 0, ?, ?, ? \
             FROM account WHERE upstream_key = ?",
        )
        .bind(format!("hash-{index}"))
        .bind(format!("request-{index}"))
        .bind(minutes_from(now, occurred))
        .bind(model)
        .bind(if failed { 429 } else { 200 })
        .bind(failed)
        .bind(cost)
        .bind(upstream_key)
        .execute(pool)
        .await?;
    }

    Ok(())
}

#[tokio::test]
async fn a_prediction_spends_the_learned_rate_and_carries_a_reset_window_forward() {
    let calls = Calls::default();
    let (client, state) = service(&calls).await.expect("a service");
    client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await
        .assert_status_is_ok();
    let now = Timestamp::now();
    let pool = &state.database.pool;
    insert_windows(pool, now).await.expect("windows");
    insert_samples(pool, now).await.expect("samples");
    insert_events(pool, now).await.expect("events");

    let listed = client
        .get("/api/quota")
        .send()
        .await
        .json()
        .await
        .value()
        .deserialize::<Value>();
    let claude = by_provider(&listed, "claude");
    let predicted = |window: &Value| window["prediction"]["used_fraction"].as_f64();
    let close = |value: Option<f64>, expected: f64| {
        value.is_some_and(|value| (value - expected).abs() < 1e-9)
    };

    let opus = window(claude, "opus");
    assert!(close(predicted(opus), 0.3), "predicted {opus}");
    assert_eq!(opus["prediction"]["has_reset"], false);
    assert!(
        close(opus["calibration"]["fraction_per_usd"].as_f64(), 0.2),
        "calibrated {opus}"
    );
    assert_eq!(opus["calibration"]["windows"], 3);
    assert_eq!(
        opus["calibration"]["outliers"].as_array().map(Vec::len),
        Some(1)
    );

    // The sonnet window reset half an hour ago and the next one began right away.
    let sonnet = window(claude, "sonnet");
    assert!(close(predicted(sonnet), 0.2), "predicted {sonnet}");
    assert_eq!(sonnet["prediction"]["has_reset"], true);
    assert_eq!(
        sonnet["prediction"]["resets_at"].as_str(),
        Some(
            now.checked_add(SignedDuration::from_mins(270))
                .expect("an instant")
                .to_string()
                .as_str()
        )
    );

    assert_eq!(window(claude, "untouched")["prediction"], Value::Null);
    assert_eq!(window(claude, "cowork")["prediction"], Value::Null);
    assert_eq!(window(claude, "cowork")["calibration"], Value::Null);

    // The five-hour window reset two hours ago and the next opened at the request 100 minutes ago.
    let codex = by_provider(&listed, "codex");
    let five_hour = window(codex, "five-hour");
    assert!(close(predicted(five_hour), 0.2), "predicted {five_hour}");
    let resets_at = five_hour["prediction"]["resets_at"]
        .as_str()
        .and_then(|at| at.parse::<Timestamp>().ok())
        .expect("a predicted reset");
    assert_eq!(resets_at.as_second() % 3600, 0);
    assert!(
        resets_at > now + SignedDuration::from_mins(140)
            && resets_at <= now + SignedDuration::from_mins(200)
    );
    let idle = window(codex, "idle");
    assert!(close(predicted(idle), 0.0), "predicted {idle}");
    assert_eq!(idle["prediction"]["resets_at"], Value::Null);

    let kimi = predicted(window(by_provider(&listed, "kimi"), "limit-0"));
    assert!(close(kimi, 0.12), "predicted {kimi:?}");
}

#[tokio::test]
async fn no_secret_reaches_a_response_or_the_database() {
    let calls = Calls::default();
    let (client, state) = service(&calls).await.expect("a service");

    let mut bodies = Vec::new();
    for (path, body) in [
        ("/api/quota/sync", json!({})),
        ("/api/quota/refresh", json!({})),
    ] {
        let response = client.post(path).body_json(&body).send().await;
        response.assert_status_is_ok();
        bodies.push(response.0.into_body().into_string().await.expect("a body"));
    }
    let listed = client.get("/api/quota").send().await;
    bodies.push(listed.0.into_body().into_string().await.expect("a body"));
    let stored = stored_text(&state).await.expect("the stored rows");

    assert!(
        calls
            .all()
            .iter()
            .any(|call| call["header"]["Authorization"] == format!("Bearer {DCA_TOKEN}")),
        "the Meta call never ran"
    );
    for secret in [MANAGEMENT_KEY, DCA_TOKEN, META_LLM_KEY, API_KEY_LABEL] {
        for body in &bodies {
            assert!(!body.contains(secret), "a response carries {secret}");
        }
        assert!(!stored.contains(secret), "the database holds {secret}");
    }
}
