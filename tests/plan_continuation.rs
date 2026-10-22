use std::sync::Arc;

use metered_usage::account::Account;
use metered_usage::app::AppState;
use metered_usage::change::Change;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::{Credentials, Token};
use metered_usage::id::Id;
use metered_usage::plan::leverage::PlanLeverage;
use metered_usage::source::Source;
use metered_usage::usage::cliproxy;
use metered_usage::usage::ingest::ingest;
use poem::Endpoint;
use poem::http::{StatusCode, header};
use poem::test::TestClient;
use serde_json::{Value, json};

struct Fixture<E> {
    state: Arc<AppState>,
    admin: TestClient<E>,
    consumer: TestClient<E>,
    previous: String,
    replacement: String,
}

impl<E: Endpoint> Fixture<E> {
    async fn accounts(&self) -> Value {
        let response = self.consumer.get("/api/accounts").send().await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize()
    }

    async fn changes(&self, cursor: i64) -> Value {
        let response = self
            .consumer
            .get(format!("/api/events?cursor={cursor}&timeout=0"))
            .send()
            .await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize()
    }
}

async fn fixture() -> Result<Fixture<impl Endpoint>, Box<dyn std::error::Error>> {
    let config = Config::parse(
        "[[source]]\nkey='local'\nkind='cliproxy'\nbase_url='http://127.0.0.1:1'\nkey_env='KEY'",
        |_| Some("test-key".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let state = Arc::new(AppState::new(
        database,
        Credentials {
            token: Token::try_from("test-admin".to_owned())?,
            read_token: None,
            consumer_token: Some(Token::try_from("test-consumer".to_owned())?),
        },
        config,
    )?);
    let source = Source::list(&state.database).await?.remove(0);
    let records = [
        ("new", "2026-10-19T12:00:00Z"),
        ("old", "2026-09-20T12:00:00Z"),
        ("old", "2026-10-01T12:00:00Z"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (key, at))| {
        cliproxy::parse(json!({
            "timestamp": at, "provider": "claude", "model": "claude-opus-4",
            "endpoint": "POST /v1/messages", "request_id": format!("request-{index}"),
            "auth_type": "oauth", "auth_index": key, "source": "luc@example.com",
            "tokens": { "input_tokens": 100, "output_tokens": 50, "total_tokens": 150 },
            "fail": { "status_code": 200 }
        }))
    })
    .collect();
    let report = ingest(&state, source.id, records).await?;
    assert_eq!((report.accepted, report.rejected), (3, 0));
    let previous: Id<Account> =
        sqlx::query_scalar("SELECT account_id FROM account WHERE upstream_key = 'old'")
            .fetch_one(&state.database.pool)
            .await?;
    let replacement: Id<Account> =
        sqlx::query_scalar("SELECT account_id FROM account WHERE upstream_key = 'new'")
            .fetch_one(&state.database.pool)
            .await?;
    for (account_id, removed) in [
        (previous, Some("2026-10-19T00:00:00.000000000Z")),
        (replacement, None),
    ] {
        sqlx::query("INSERT INTO credential_state (account_id, plan, soft_observed_at, removed_at) VALUES (?, 'max', '2026-10-20T00:00:00.000000000Z', ?)")
            .bind(account_id).bind(removed).execute(&state.database.pool).await?;
    }
    let admin = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, "Bearer test-admin");
    let consumer = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, "Bearer test-consumer");
    admin
        .post("/api/plans")
        .body_json(&json!({
            "account_id": previous.encode(), "name": "Max", "monthly_usd": 180.0,
            "period_start": "2026-09-18T00:00:00Z"
        }))
        .send()
        .await
        .assert_status_is_ok();
    Ok(Fixture {
        state,
        admin,
        consumer,
        previous: previous.encode(),
        replacement: replacement.encode(),
    })
}

#[tokio::test]
async fn only_removed_matching_credentials_with_a_tracked_plan_suggest_continuing() {
    for (change, should_suggest) in [
        ("SELECT 1", true),
        ("UPDATE account SET display_name = 'Work plan'", true),
        (
            "UPDATE account SET label = ' LUC@example.com ' WHERE upstream_key = 'new'",
            true,
        ),
        (
            "UPDATE account SET provider = 'codex' WHERE upstream_key = 'new'",
            false,
        ),
        (
            "UPDATE account SET first_seen_at = '2026-09-01T00:00:00.000000000Z' WHERE upstream_key = 'new'",
            false,
        ),
        (
            "UPDATE account SET first_seen_at = '2026-09-20T12:00:00.000000000Z' WHERE upstream_key = 'new'",
            false,
        ),
        (
            "INSERT INTO source (source_id, key, name, kind, base_url, created_at) VALUES (1, 'elsewhere', 'Elsewhere', 'cliproxy', 'http://127.0.0.1:1', '2026-01-01T00:00:00.000000000Z'); UPDATE account SET source_id = 1 WHERE upstream_key = 'new'",
            false,
        ),
        (
            "UPDATE account SET label = 'other@example.com' WHERE upstream_key = 'new'",
            false,
        ),
        ("UPDATE account SET label = 'not-an-email'", false),
        (
            "UPDATE account SET auth_kind = 'api_key' WHERE upstream_key = 'new'",
            false,
        ),
        (
            "UPDATE credential_state SET plan = 'pro' WHERE removed_at IS NULL",
            false,
        ),
        ("UPDATE credential_state SET plan = ''", false),
        ("UPDATE credential_state SET removed_at = NULL", false),
        (
            "UPDATE credential_state SET removed_at = '2026-10-20T00:00:00.000000000Z'",
            false,
        ),
        (
            "UPDATE credential_state SET disabled = 1 WHERE removed_at IS NULL",
            false,
        ),
        ("DELETE FROM plan", false),
        (
            "INSERT INTO plan (plan_id, account_id, name, monthly_usd, period_start) SELECT 1, account_id, 'Max', 180, '2026-10-19T00:00:00.000000000Z' FROM account WHERE upstream_key = 'new'",
            false,
        ),
    ] {
        let fixture = fixture().await.unwrap();
        sqlx::query(change)
            .execute(&fixture.state.database.pool)
            .await
            .unwrap();
        let accounts = fixture.accounts().await;
        let previous = accounts["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|account| account["account_id"] == fixture.previous)
            .unwrap();
        let expected = if should_suggest {
            json!([fixture.replacement])
        } else {
            json!([])
        };
        assert_eq!(
            previous["plan_continuation_account_ids"], expected,
            "{change}"
        );
        assert_eq!(
            accounts["accounts"].as_array().unwrap().len(),
            2,
            "matching never merges"
        );
    }
}

#[tokio::test]
async fn dismissal_is_pair_specific_persistent_and_notifies_consumers_once() {
    let fixture = fixture().await.unwrap();
    let third = fixture.state.database.ids.next::<Account>();
    sqlx::query("INSERT INTO account (account_id, source_id, provider, upstream_key, auth_kind, label, first_seen_at, last_seen_at) SELECT ?, source_id, provider, 'third', auth_kind, label, first_seen_at, last_seen_at FROM account WHERE upstream_key = 'new'")
        .bind(third).execute(&fixture.state.database.pool).await.unwrap();
    sqlx::query("INSERT INTO credential_state (account_id, plan, soft_observed_at) VALUES (?, 'max', '2026-10-20T00:00:00.000000000Z')")
        .bind(third).execute(&fixture.state.database.pool).await.unwrap();
    let before = fixture.consumer.get("/api/accounts").send().await;
    let etag = before.0.headers()[header::ETAG]
        .to_str()
        .unwrap()
        .to_owned();
    let cursor = Change::latest(&fixture.state.database).await.unwrap();
    let path = format!("/api/accounts/{}/different-plan", fixture.previous);
    fixture
        .consumer
        .post(&path)
        .body_json(&json!({ "other_account_id": fixture.replacement }))
        .send()
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    fixture
        .admin
        .post(&path)
        .body_json(&json!({ "other_account_id": fixture.replacement }))
        .send()
        .await
        .assert_status(StatusCode::NO_CONTENT);
    fixture
        .consumer
        .get("/api/accounts")
        .header(header::IF_NONE_MATCH, etag)
        .send()
        .await
        .assert_status_is_ok();
    let changes = fixture.changes(cursor).await;
    let events = changes["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| event["kind"] == "account.changed")
    );
    let mut changed_ids: Vec<_> = events
        .iter()
        .map(|event| event["account_id"].as_str().unwrap())
        .collect();
    changed_ids.sort_unstable();
    let mut expected_ids = vec![fixture.previous.as_str(), fixture.replacement.as_str()];
    expected_ids.sort_unstable();
    assert_eq!(changed_ids, expected_ids);
    fixture
        .admin
        .post(format!(
            "/api/accounts/{}/different-plan",
            fixture.replacement
        ))
        .body_json(&json!({ "other_account_id": fixture.previous }))
        .send()
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(
        Change::latest(&fixture.state.database).await.unwrap(),
        changes["cursor"].as_i64().unwrap()
    );
    sqlx::query("UPDATE credential_state SET removed_at = NULL WHERE removed_at IS NOT NULL")
        .execute(&fixture.state.database.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE credential_state SET removed_at = '2026-10-21T00:00:00.000000000Z' WHERE account_id = ?")
        .bind(fixture.previous.parse::<Id<Account>>().unwrap()).execute(&fixture.state.database.pool).await.unwrap();
    let accounts = fixture.accounts().await;
    let previous = accounts["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["account_id"] == fixture.previous)
        .unwrap();
    assert_eq!(
        previous["plan_continuation_account_ids"],
        json!([third.encode()])
    );
    let plans = fixture
        .admin
        .get("/api/plans")
        .send()
        .await
        .json()
        .await
        .value()
        .deserialize::<Value>();
    assert!(
        plans["plans"][0].get("period_end").is_none(),
        "dismissal leaves the old plan open"
    );
}

#[tokio::test]
async fn continuing_keeps_history_price_dates_and_billing_periods_including_an_end_date() {
    for has_ended in [false, true] {
        let fixture = fixture().await.unwrap();
        if has_ended {
            sqlx::query("UPDATE plan SET period_end = '2026-10-20T00:00:00.000000000Z'")
                .execute(&fixture.state.database.pool)
                .await
                .unwrap();
        }
        let now = "2026-10-20T12:00:00Z".parse().unwrap();
        let before = PlanLeverage::list(&fixture.state.database, 24, now)
            .await
            .unwrap()
            .remove(0);
        let cursor = Change::latest(&fixture.state.database).await.unwrap();
        fixture
            .admin
            .post(format!("/api/accounts/{}/merge", fixture.previous))
            .body_json(&json!({ "into_account_id": fixture.replacement, "require_plan_continuation": true }))
            .send()
            .await
            .assert_status_is_ok();
        let accounts = fixture.accounts().await;
        assert_eq!(accounts["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(accounts["accounts"][0]["account_id"], fixture.replacement);
        assert_eq!(accounts["accounts"][0]["event_count"], 3);
        assert_eq!(
            accounts["accounts"][0]["credentials"][0]["account"]["account_id"],
            fixture.previous
        );
        assert_eq!(
            accounts["accounts"][0]["plan_continuation_account_ids"],
            json!([])
        );
        let after = PlanLeverage::list(&fixture.state.database, 24, now)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(after.plan.id, before.plan.id);
        assert_eq!(after.plan.name, before.plan.name);
        assert_eq!(after.plan.monthly_usd.to_bits(), 180.0_f64.to_bits());
        assert_eq!(after.plan.period_start, before.plan.period_start);
        assert_eq!(after.plan.period_end, before.plan.period_end);
        assert_eq!(after.plan.account.id.encode(), fixture.replacement);
        assert_eq!(after.periods.len(), 2);
        for (old, new) in before.periods.iter().zip(&after.periods) {
            assert_eq!(
                (new.start, new.end, new.complete),
                (old.start, old.end, old.complete)
            );
        }
        assert_eq!(
            after
                .periods
                .iter()
                .map(|period| period.requests)
                .sum::<i64>(),
            3
        );
        assert_eq!(
            after
                .periods
                .iter()
                .map(|period| period.total_tokens)
                .sum::<i64>(),
            450
        );
        fixture
            .admin
            .post(format!("/api/accounts/{}/merge", fixture.previous))
            .body_json(&json!({ "into_account_id": fixture.replacement }))
            .send()
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        fixture
            .admin
            .post(format!("/api/accounts/{}/different-plan", fixture.previous))
            .body_json(&json!({ "other_account_id": fixture.replacement }))
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
        let changes = fixture.changes(cursor).await;
        let events = changes["events"].as_array().unwrap();
        assert!(
            events.iter().any(|event| event["kind"] == "plan.changed"
                && event["plan_id"] == after.plan.id.encode())
        );
        for account_id in [&fixture.previous, &fixture.replacement] {
            assert!(
                events.iter().any(|event| event["kind"] == "account.changed"
                    && event["account_id"] == *account_id)
            );
        }
    }
}

#[tokio::test]
async fn a_stale_continuation_cannot_combine_two_plans_or_override_a_dismissal() {
    for change in [
        "INSERT INTO plan (plan_id, account_id, name, monthly_usd, period_start) SELECT 1, account_id, 'Another Max', 100, '2026-10-19T00:00:00.000000000Z' FROM account WHERE upstream_key = 'new'",
        "INSERT INTO plan_continuation_dismissal (account_low, account_high) SELECT MIN(account_id), MAX(account_id) FROM account",
        "UPDATE credential_state SET removed_at = NULL",
        "UPDATE credential_state SET plan = 'pro' WHERE removed_at IS NULL",
    ] {
        let fixture = fixture().await.unwrap();
        sqlx::query(change)
            .execute(&fixture.state.database.pool)
            .await
            .unwrap();
        let cursor = Change::latest(&fixture.state.database).await.unwrap();
        fixture.admin.post(format!("/api/accounts/{}/merge", fixture.previous))
            .body_json(&json!({ "into_account_id": fixture.replacement, "require_plan_continuation": true }))
            .send().await.assert_status(StatusCode::BAD_REQUEST);
        let old = Account::load(&fixture.state.database, fixture.previous.parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(old.merged_into_account_id.is_none(), "{change}");
        assert_eq!(old.event_count, 2);
        let accounts = fixture.accounts().await;
        assert_eq!(accounts["accounts"].as_array().unwrap().len(), 2);
        assert_eq!(
            Change::latest(&fixture.state.database).await.unwrap(),
            cursor,
            "{change}"
        );
    }
}
