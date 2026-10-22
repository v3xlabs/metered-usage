use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use metered_usage::account::{Account, AuthKind, NewAccount};
use metered_usage::app::AppState;
use metered_usage::change::Change;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::quota::response;
use metered_usage::quota::window::{NewWindow, QuotaWindow};
use metered_usage::source::Source;
use poem::Endpoint;
use poem::http::header;
use poem::test::TestClient;
use serde_json::{Value, json};

const TOKEN: &str = "meta-quota-test";
const BODY: &str = r#"{"type":"error","error":{"type":"rate_limit_error","message":"Subscription quota exhausted. Your usage window resets at 2026-10-26T00:00:00Z."}}"#;
type Failure = Box<dyn std::error::Error>;

async fn service() -> Result<(Arc<AppState>, TestClient<impl Endpoint>, String), Failure> {
    let config = Config::parse(
        "[[source]]\nkey = 'local'\nkind = 'cliproxy'\nbase_url = 'http://127.0.0.1:1'\nkey_env = 'PROXY_KEY'",
        |_| Some("test-key".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let source = Source::list(&database).await?.remove(0);
    let mut transaction = database.write().await?;
    let account = Account::resolve(
        &mut transaction,
        database.ids.next(),
        source.id,
        "meta",
        &NewAccount::new(
            "meta-one".to_owned(),
            AuthKind::OAuth,
            Some("mo@example.com"),
        ),
        Timestamp::now(),
    )
    .await?;
    sqlx::query("INSERT INTO credential_state (account_id) VALUES (?)")
        .bind(account)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    let state = Arc::new(AppState::new(
        database,
        Token::try_from(TOKEN.to_owned())?.into(),
        config,
    )?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    Ok((state, client, source.id.to_string()))
}

fn record(at: Timestamp, status: i64, body: &str) -> Value {
    let body = body.replace(
        "2026-10-26T00:00:00Z",
        &(at + SignedDuration::from_hours(24 * 7)).to_string(),
    );
    json!({
        "timestamp": at.to_string(), "provider": "meta", "model": "muse-wonder-full-1.0",
        "endpoint": "POST /v1/responses", "auth_index": "meta-one", "auth_type": "oauth",
        "source": "mo@example.com", "tokens": {}, "failed": status >= 400,
        "fail": { "status_code": status, "body": body },
    })
}

async fn post(client: &TestClient<impl Endpoint>, source: &str, records: Vec<Value>) {
    let result = client
        .post(format!("/api/sources/{source}/events"))
        .body_json(&json!({ "records": records }))
        .send()
        .await;
    result.assert_status_is_ok();
    assert_eq!(
        result.json().await.value().deserialize::<Value>()["rejected"],
        0
    );
}

async fn quota(client: &TestClient<impl Endpoint>) -> Value {
    let result = client.get("/api/quota").send().await;
    result.assert_status_is_ok();
    let body: Value = result.json().await.value().deserialize();
    body["accounts"][0].clone()
}

async fn changes(client: &TestClient<impl Endpoint>, cursor: i64) -> Value {
    let result = client
        .get(format!(
            "/api/events?cursor={cursor}&kinds=quota.updated&timeout=0"
        ))
        .send()
        .await;
    result.assert_status_is_ok();
    result.json().await.value().deserialize()
}

#[tokio::test]
async fn meta_429_reports_exhaustion_and_reset_to_consumers() -> Result<(), Failure> {
    let (state, client, source) = service().await?;
    let cursor = Change::latest(&state.database).await?;
    let at = Timestamp::now();
    post(&client, &source, vec![record(at, 429, BODY)]).await;
    let account = quota(&client).await;
    let window = &account["windows"][0];
    assert_eq!(window["window_key"], response::WINDOW_KEY);
    assert_eq!(window["used_fraction"], 1.0);
    assert_eq!(
        window["resets_at"],
        (at + SignedDuration::from_hours(24 * 7)).to_string()
    );
    assert!(window.get("window_seconds").is_none());
    assert!(window.get("used_value").is_none());
    assert!(window.get("prediction").is_none());
    let feed = changes(&client, cursor).await;
    assert_eq!(feed["events"][0]["kind"], "quota.updated");
    assert_eq!(
        feed["events"][0]["account_id"],
        account["account"]["account_id"]
    );
    assert_eq!(feed["events"][0]["window_key"], response::WINDOW_KEY);
    Ok(())
}

#[tokio::test]
async fn later_success_clears_exhaustion_without_erasing_measured_windows() -> Result<(), Failure> {
    let (state, client, source) = service().await?;
    let at = Timestamp::now();
    let failed = record(at, 429, BODY);
    post(&client, &source, vec![failed.clone()]).await;
    let account = quota(&client).await["account"]["account_id"]
        .as_str()
        .unwrap()
        .parse()?;
    let mut transaction = state.database.write().await?;
    QuotaWindow::replace(
        &mut transaction,
        account,
        &[NewWindow {
            key: "weekly".to_owned(),
            label: "Weekly".to_owned(),
            used_fraction: Some(0.4),
            ..NewWindow::default()
        }],
        at,
    )
    .await?;
    transaction.commit().await?;
    assert_eq!(quota(&client).await["windows"].as_array().unwrap().len(), 2);
    let cursor = Change::latest(&state.database).await?;
    post(
        &client,
        &source,
        vec![record(at + SignedDuration::from_secs(1), 200, "")],
    )
    .await;
    let windows = quota(&client).await["windows"].clone();
    assert_eq!(windows.as_array().unwrap().len(), 1);
    assert_eq!(windows[0]["window_key"], "weekly");
    assert_eq!(windows[0]["used_fraction"], 0.4);
    assert_eq!(
        changes(&client, cursor).await["events"][0]["window_key"],
        response::WINDOW_KEY
    );
    // A duplicate and an older, previously unseen failure cannot resurrect the marker.
    post(
        &client,
        &source,
        vec![failed, record(at - SignedDuration::from_secs(1), 429, BODY)],
    )
    .await;
    assert_eq!(quota(&client).await["windows"], windows);
    // A new exhaustion follows a measured row in storage, but leads the consumer's list.
    post(
        &client,
        &source,
        vec![record(at + SignedDuration::from_secs(2), 429, BODY)],
    )
    .await;
    let exhausted = quota(&client).await;
    assert_eq!(exhausted["windows"][0]["window_key"], response::WINDOW_KEY);
    assert_eq!(exhausted["windows"][1]["window_key"], "weekly");
    Ok(())
}

#[tokio::test]
async fn reset_clears_exhaustion_and_records_the_same_change_kind() -> Result<(), Failure> {
    let (state, client, source) = service().await?;
    let at = Timestamp::now();
    post(&client, &source, vec![record(at, 429, BODY)]).await;
    let cursor = Change::latest(&state.database).await?;
    response::expire(&state.database, at + SignedDuration::from_hours(24 * 7)).await?;
    assert_eq!(quota(&client).await["windows"], json!([]));
    assert_eq!(
        changes(&client, cursor).await["events"][0]["window_key"],
        response::WINDOW_KEY
    );
    Ok(())
}

#[tokio::test]
async fn unrelated_failures_and_older_success_do_not_clear_exhaustion() -> Result<(), Failure> {
    let (_state, client, source) = service().await?;
    let at = Timestamp::now();
    post(&client, &source, vec![record(at, 429, BODY)]).await;
    let windows = quota(&client).await["windows"].clone();
    post(
        &client,
        &source,
        vec![
            record(at - SignedDuration::from_secs(1), 200, ""),
            record(at + SignedDuration::from_secs(1), 500, "server error"),
            record(
                at + SignedDuration::from_secs(2),
                429,
                r#"{"error":{"type":"rate_limit_error","message":"Too many requests"}}"#,
            ),
        ],
    )
    .await;
    assert_eq!(quota(&client).await["windows"], windows);
    Ok(())
}

#[tokio::test]
async fn other_providers_malformed_dates_and_past_resets_are_not_meta_exhaustion()
-> Result<(), Failure> {
    let (_state, client, source) = service().await?;
    let at = Timestamp::now();
    let mut other = record(at, 429, BODY);
    other["provider"] = json!("claude");
    post(
        &client,
        &source,
        vec![
            other,
            record(
                at + SignedDuration::from_secs(1),
                429,
                &BODY.replace("2026-10-26T00:00:00Z", "not-a-date"),
            ),
            record(
                at + SignedDuration::from_secs(2),
                429,
                &BODY.replace("2026-10-26T00:00:00Z", "2020-10-26T00:00:00Z"),
            ),
        ],
    )
    .await;
    assert_eq!(quota(&client).await["windows"], json!([]));
    Ok(())
}

#[tokio::test]
async fn supplied_429_body_becomes_an_exhausted_window_at_its_reset() -> Result<(), Failure> {
    let at: Timestamp = "2026-10-20T21:55:00Z".parse()?;
    let reset: Timestamp = "2026-10-26T00:00:00Z".parse()?;
    let message: Value = serde_json::from_str(BODY)?;
    let mut malformed_reset = message.clone();
    malformed_reset["error"]["resets_at"] = json!({"invalid": true});
    let mut oversized_reset = message.clone();
    oversized_reset["error"]["resets_at"] = json!(i64::MAX);
    let mut fractional_reset = message.clone();
    fractional_reset["error"]["resets_at"] = json!(1.5);
    for body in [
        message,
        json!({"error": {"code": "rate_limit_exceeded", "resets_at": reset.as_second()}}),
        json!({"error": {"message": "SUBSCRIPTION QUOTA EXHAUSTED", "resets_at": reset.as_second().to_string()}}),
        malformed_reset,
        oversized_reset,
        fractional_reset,
    ] {
        let (state, _client, _source) = service().await?;
        let mut raw = record(at, 429, BODY);
        raw["fail"]["body"] = json!(body.to_string());
        let event = metered_usage::usage::NewUsageEvent::try_from(serde_json::from_value::<
            metered_usage::usage::cliproxy::CliProxyRecord,
        >(raw)?)?;
        let account = sqlx::query_scalar("SELECT account_id FROM account")
            .fetch_one(&state.database.pool)
            .await?;
        let mut transaction = state.database.write().await?;
        response::observe(&mut transaction, account, &event, at).await?;
        transaction.commit().await?;
        let window: (f64, String) = sqlx::query_as(
            "SELECT used_fraction, resets_at FROM quota_window WHERE account_id = ?",
        )
        .bind(account)
        .fetch_one(&state.database.pool)
        .await?;
        assert_eq!(
            window,
            (1.0, "2026-10-26T00:00:00.000000000Z".to_owned()),
            "{body}"
        );
    }
    Ok(())
}
