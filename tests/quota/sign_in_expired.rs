use metered_usage::quota::sign_in::Reauthentication;
use metered_usage::source::Source;
use poem::test::TestClient;
use serde_json::{Value, json};

use super::{Calls, accounts_of, by_provider, service, stored_text};

const EXPIRED: &str = "OAuth access token has expired. Re-authenticate to continue.";

async fn refresh_one(client: &TestClient<impl poem::Endpoint>, account_id: &Value) -> Value {
    let response = client
        .post("/api/quota/refresh")
        .body_json(&json!({ "account_id": account_id }))
        .send()
        .await;
    response.assert_status_is_ok();
    response.json().await.value().deserialize::<Value>()
}

async fn cursor(client: &TestClient<impl poem::Endpoint>) -> i64 {
    let response = client.get("/api/events").send().await;
    response.assert_status_is_ok();
    let cursor = response.json().await.value().deserialize::<Value>()["cursor"].as_i64();
    assert!(cursor.is_some());
    cursor.unwrap_or_default()
}

async fn assert_changed(client: &TestClient<impl poem::Endpoint>, cursor: i64, account_id: &Value) {
    let response = client
        .get(format!("/api/events?cursor={cursor}&timeout=0"))
        .send()
        .await;
    response.assert_status_is_ok();
    let changes = response.json().await.value().deserialize::<Value>();
    assert!(
        changes["events"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|event| event["kind"] == "account.changed" && &event["account_id"] == account_id),
        "{changes}"
    );
}

#[tokio::test]
async fn expired_sign_in_keeps_reported_quota_until_a_successful_read() {
    let calls = Calls::default();
    let (client, state) = service(&calls).await.expect("service");
    let response = client
        .post("/api/quota/refresh")
        .body_json(&json!({}))
        .send()
        .await;
    response.assert_status_is_ok();
    let first = response.json().await.value().deserialize::<Value>();
    let claude = by_provider(&first, "claude");
    let account_id = &claude["account"]["account_id"];
    assert!(claude.get("sign_in_error").is_none());
    let windows = claude["windows"].clone();
    let before = cursor(&client).await;

    *calls.claude_response.lock().expect("response") = Some((
        401,
        json!({
            "type": "error", "error": { "type": "authentication_error", "message": EXPIRED }
        }),
    ));
    let expired = refresh_one(&client, account_id).await;
    assert_eq!(expired["failed"], 1);
    let claude = by_provider(&expired, "claude");
    assert_eq!(claude["sign_in_error"], "expired");
    assert_eq!(claude["windows"], windows);
    let source = Source::list(&state.database)
        .await
        .expect("sources")
        .remove(0);
    assert_eq!(
        claude["reauthentication"]["url"],
        format!("{}/management.html#/oauth", source.base_url)
    );
    assert_changed(&client, before, account_id).await;

    let listed = accounts_of(&client).await;
    let account = listed["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|account| &account["account_id"] == account_id)
        .expect("account");
    assert_eq!(account["sign_in_error"], "expired");
    assert_eq!(account["reauthentication"], claude["reauthentication"]);
    let read = client.get("/api/quota").send().await;
    read.assert_status_is_ok();
    assert_eq!(
        by_provider(&read.json().await.value().deserialize::<Value>(), "claude")["sign_in_error"],
        "expired"
    );

    // Neither a fresh credential listing nor a different failed read proves recovery.
    client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await
        .assert_status_is_ok();
    *calls.claude_response.lock().expect("response") =
        Some((503, json!({ "error": "overloaded" })));
    metered_usage::quota::recheck_expired(&state).await;
    assert_eq!(
        by_provider(&refresh_one(&client, account_id).await, "claude")["sign_in_error"],
        "expired"
    );

    // The next check after a new sign-in clears it with nobody pressing refresh.
    *calls.claude_response.lock().expect("response") = None;
    let before = cursor(&client).await;
    metered_usage::quota::recheck_expired(&state).await;
    let quota = client
        .get("/api/quota")
        .send()
        .await
        .json()
        .await
        .value()
        .deserialize::<Value>();
    let claude = by_provider(&quota, "claude");
    assert!(claude.get("sign_in_error").is_none());
    assert!(claude.get("reauthentication").is_none());
    assert!(claude.get("hard_refresh_error").is_none());
    assert_eq!(claude["windows"].as_array().map(Vec::len), Some(2));
    assert_changed(&client, before, account_id).await;
    let listed = accounts_of(&client).await;
    let account = listed["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|account| &account["account_id"] == account_id)
        .expect("account");
    assert!(account.get("sign_in_error").is_none());
    assert!(account.get("reauthentication").is_none());
}

#[tokio::test]
async fn a_plain_text_expired_token_answer_is_the_same_state() {
    let calls = Calls::default();
    let (client, _state) = service(&calls).await.expect("service");
    let synced = client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await;
    synced.assert_status_is_ok();
    let quota = synced.json().await.value().deserialize::<Value>();
    let account_id = &by_provider(&quota, "claude")["account"]["account_id"];
    *calls.claude_response.lock().expect("response") = Some((401, json!(EXPIRED)));
    let expired = refresh_one(&client, account_id).await;
    assert_eq!(by_provider(&expired, "claude")["sign_in_error"], "expired");
}

/// A credential turned off in CLIProxy cannot be refreshed, so the recheck leaves it alone
/// and only touches the one whose sign-in expired.
#[tokio::test]
async fn the_recheck_reads_only_enabled_credentials_whose_sign_in_expired() {
    let calls = Calls::default();
    let (client, state) = service(&calls).await.expect("service");
    let refreshed = client
        .post("/api/quota/refresh")
        .body_json(&json!({}))
        .send()
        .await;
    refreshed.assert_status_is_ok();
    let quota = refreshed.json().await.value().deserialize::<Value>();
    let account_id = &by_provider(&quota, "claude")["account"]["account_id"];
    *calls.claude_response.lock().expect("response") = Some((401, json!(EXPIRED)));
    refresh_one(&client, account_id).await;
    *calls.claude_response.lock().expect("response") = None;

    calls.disable(&["a1"]);
    client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await
        .assert_status_is_ok();
    let sent = calls.all().len();
    metered_usage::quota::recheck_expired(&state).await;
    assert_eq!(calls.all().len(), sent, "a turned-off credential was read");
    let still = accounts_of(&client).await;
    assert_eq!(
        by_provider_account(&still, account_id)["sign_in_error"],
        "expired"
    );

    calls.disable(&[]);
    // The timer syncs before it rechecks, which is what marks the credential refreshable again.
    client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await
        .assert_status_is_ok();
    let sent = calls.all().len();
    metered_usage::quota::recheck_expired(&state).await;
    let reads = calls.all()[sent..]
        .iter()
        .filter(|call| call["url"] == "https://api.anthropic.com/api/oauth/usage")
        .count();
    assert_eq!(reads, 1, "only the expired credential is read, once");
    let cleared = accounts_of(&client).await;
    assert!(
        by_provider_account(&cleared, account_id)
            .get("sign_in_error")
            .is_none()
    );
}

/// Null when absent, so the assertion that reads it fails.
fn by_provider_account<'a>(accounts: &'a Value, account_id: &Value) -> &'a Value {
    accounts["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|account| &account["account_id"] == account_id)
        .unwrap_or(&Value::Null)
}

#[tokio::test]
async fn other_failures_do_not_claim_expiry_and_error_bodies_are_not_stored() {
    let calls = Calls::default();
    let (client, state) = service(&calls).await.expect("service");
    client
        .post("/api/quota/sync")
        .body_json(&json!({}))
        .send()
        .await
        .assert_status_is_ok();
    let quota = client
        .get("/api/quota")
        .send()
        .await
        .json()
        .await
        .value()
        .deserialize::<Value>();
    let account_id = &by_provider(&quota, "claude")["account"]["account_id"];
    for (status, body) in [
        (
            401,
            json!({ "error": { "message": "invalid API key SECRET-ERROR-123" } }),
        ),
        (403, json!({ "error": { "message": EXPIRED } })),
        (
            401,
            json!({ "error": { "message": "OAuth access token has not expired" } }),
        ),
    ] {
        *calls.claude_response.lock().expect("response") = Some((status, body));
        let failed = refresh_one(&client, account_id).await;
        let claude = by_provider(&failed, "claude");
        assert!(claude.get("sign_in_error").is_none(), "{failed}");
        assert!(claude.get("reauthentication").is_none());
        assert_eq!(
            claude["hard_refresh_error"],
            format!("the provider answered {status}")
        );
    }
    assert!(
        !stored_text(&state)
            .await
            .expect("database text")
            .contains("SECRET-ERROR-123")
    );
}

#[test]
fn reauthentication_stays_on_the_source_without_url_credentials() {
    let action =
        Reauthentication::new("https://proxy.example/prefix/?secret=discard#old", "claude")
            .expect("known provider");
    assert_eq!(
        action.url,
        "https://proxy.example/prefix/management.html#/oauth"
    );
    for base_url in [
        "https://user:secret@proxy.example",
        "javascript:alert(1)",
        "not a URL",
    ] {
        assert!(Reauthentication::new(base_url, "claude").is_none());
    }
    assert!(Reauthentication::new("https://proxy.example", "unknown").is_none());
}
