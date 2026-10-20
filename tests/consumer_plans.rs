use std::sync::Arc;

use metered_usage::account::Account;
use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::{Credentials, Token};
use metered_usage::id::Id;
use metered_usage::plan::Plan;
use metered_usage::price::{ModelPrice, NewPrice, Origin, Rates};
use metered_usage::source::Source;
use metered_usage::usage::cliproxy;
use metered_usage::usage::ingest::ingest;
use poem::Endpoint;
use poem::http::{StatusCode, header};
use poem::test::{TestClient, TestResponse};
use serde_json::{Value, json};

const ADMIN: &str = "test-admin-token-not-a-secret";
const CONSUMER: &str = "test-consumer-token-not-a-secret";
const MODEL: &str = "consumer-plan-model";
const TOLERANCE: f64 = 1e-9;

type Failure = Box<dyn std::error::Error>;

struct Service<E> {
    state: Arc<AppState>,
    /// Presents the token that admits every operation.
    admin: TestClient<E>,
    /// Presents the consumer token.
    consumer: TestClient<E>,
}

impl<E: Endpoint> Service<E> {
    /// Stores the records as the configured gateway's and prices them.
    async fn use_tokens(&self, records: Vec<Value>) -> Result<(), Failure> {
        let count = i64::try_from(records.len())?;
        let source = Source::list(&self.state.database)
            .await?
            .into_iter()
            .next()
            .ok_or("a configured source")?;
        let report = ingest(
            &self.state,
            source.id,
            records.into_iter().map(cliproxy::parse).collect(),
        )
        .await?;
        assert_eq!((report.accepted, report.rejected), (count, 0));
        ModelPrice::fill(&self.state).await?;
        Ok(())
    }

    async fn account_id(&self) -> Result<String, Failure> {
        let listed = self.admin.get("/api/accounts").send().await;
        listed.assert_status_is_ok();
        let accounts = listed.json().await.value().deserialize::<Value>();
        Ok(accounts["accounts"][0]["account_id"]
            .as_str()
            .ok_or("an account")?
            .to_owned())
    }

    /// Adds the plan of the fixture: 0.40 USD a month for January and February 2026.
    async fn create_plan(&self, account_id: &str) -> Result<String, Failure> {
        let created = self
            .admin
            .post("/api/plans")
            .body_json(&json!({
                "account_id": account_id,
                "name": "Max",
                "monthly_usd": 0.4,
                "period_start": "2026-01-01T00:00:00Z",
                "period_end": "2026-03-01T00:00:00Z",
            }))
            .send()
            .await;
        created.assert_status_is_ok();
        let plan = created.json().await.value().deserialize::<Value>();
        Ok(plan["plan_id"].as_str().ok_or("a plan id")?.to_owned())
    }

    /// The cursor a consumer that has read nothing yet starts from.
    async fn latest(&self) -> Result<i64, Failure> {
        let answer = self.consumer.get("/api/events").send().await;
        answer.assert_status_is_ok();
        let answer = answer.json().await.value().deserialize::<Value>();
        Ok(answer["cursor"].as_i64().ok_or("a cursor")?)
    }

    async fn changes(&self, cursor: i64, kinds: &str) -> Value {
        let response = self
            .consumer
            .get(format!(
                "/api/events?cursor={cursor}&kinds={kinds}&timeout=0"
            ))
            .send()
            .await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize()
    }

    async fn plans(&self) -> Value {
        let response = self.admin.get("/api/plans").send().await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize()
    }

    /// A consumer read that sends back the tag of an earlier one.
    async fn read_unless(&self, path: &str, tag: &str) -> TestResponse {
        self.consumer
            .get(path)
            .header(header::IF_NONE_MATCH, tag)
            .send()
            .await
    }
}

async fn service() -> Result<Service<impl Endpoint>, Failure> {
    let config = Config::parse(
        "[[source]]\nkey = 'local'\nkind = 'cliproxy'\nbase_url = 'http://127.0.0.1:1'\nkey_env = 'PROXY_KEY'",
        |_| Some("test-key".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let credentials = Credentials {
        token: Token::try_from(ADMIN.to_owned())?,
        read_token: None,
        consumer_token: Some(Token::try_from(CONSUMER.to_owned())?),
    };
    let state = Arc::new(AppState::new(database, credentials, config)?);
    ModelPrice::insert(
        &state.database,
        &NewPrice {
            provider: None,
            model: MODEL.to_owned(),
            service_tier: None,
            min_input_tokens: 0,
            effective_from: "2025-01-01T00:00:00Z".parse()?,
            rates: Rates {
                input_usd_per_mtok: 2.0,
                cached_input_usd_per_mtok: 0.0,
                cache_write_usd_per_mtok: 0.0,
                output_usd_per_mtok: 4.0,
            },
        },
        Origin::Manual,
    )
    .await?;
    let admin = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {ADMIN}"));
    let consumer = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {CONSUMER}"));

    Ok(Service {
        state,
        admin,
        consumer,
    })
}

/// One request of 100k input and 50k output tokens, which lists at 0.40 USD.
fn record(id: &str, timestamp: &str) -> Value {
    json!({
        "timestamp": timestamp, "provider": "openai", "model": MODEL,
        "endpoint": "POST /v1/chat/completions", "request_id": id,
        "auth_type": "api-key", "source": "sk-plan-account-one",
        "tokens": { "input_tokens": 100_000, "output_tokens": 50_000, "total_tokens": 150_000 },
        "fail": { "status_code": 200 }
    })
}

/// Two requests in January, one in February, and one on each side of the plan.
fn fixture() -> Vec<Value> {
    vec![
        record("before", "2025-12-31T12:00:00Z"),
        record("january-a", "2026-01-10T00:00:00Z"),
        record("january-b", "2026-01-20T00:00:00Z"),
        record("february", "2026-02-10T00:00:00Z"),
        record("after", "2026-03-05T00:00:00Z"),
    ]
}

fn tag(response: &TestResponse) -> String {
    response
        .0
        .headers()
        .get(header::ETAG)
        .and_then(|tag| tag.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn close(actual: &Value, expected: f64) {
    assert!(
        actual
            .as_f64()
            .is_some_and(|actual| (actual - expected).abs() < TOLERANCE),
        "{actual} is not {expected}"
    );
}

/// The kind and plan of each change, after asserting a plan change names nothing else.
fn plan_changes(answer: &Value) -> Vec<(String, String)> {
    answer["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|change| {
            let mut fields = change
                .as_object()
                .map(|fields| fields.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            fields.sort();
            assert_eq!(
                fields,
                ["cursor", "kind", "plan_id", "recorded_at"],
                "{change}"
            );
            (
                change["kind"].as_str().unwrap_or_default().to_owned(),
                change["plan_id"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_consumer_reads_plan_prices_periods_and_leverage() -> Result<(), Failure> {
    let service = service().await?;
    service.use_tokens(fixture()).await?;
    let account_id = service.account_id().await?;
    let plan_id = service.create_plan(&account_id).await?;

    let listed = service.consumer.get("/api/plans").send().await;
    listed.assert_status_is_ok();
    let listed = listed.json().await.value().deserialize::<Value>();
    let plans = listed["plans"].as_array().ok_or("a plan list")?;
    assert_eq!(plans.len(), 1, "{listed}");
    let plan = &plans[0];
    assert_eq!(plan["plan_id"], plan_id);
    assert_eq!(plan["account"]["account_id"], account_id);
    assert_eq!(plan["name"], "Max");
    close(&plan["monthly_usd"], 0.4);
    assert_eq!(plan["period_start"], "2026-01-01T00:00:00Z");
    assert_eq!(plan["period_end"], "2026-03-01T00:00:00Z");

    let read = service.consumer.get("/api/leverage").send().await;
    read.assert_status_is_ok();
    let leverage = read.json().await.value().deserialize::<Value>();
    let plans = leverage["plans"].as_array().ok_or("a leverage list")?;
    assert_eq!(plans.len(), 1, "{leverage}");
    assert_eq!(plans[0]["plan"]["plan_id"], plan_id);
    let periods = plans[0]["periods"].as_array().ok_or("periods")?;
    let [february, january] = periods.as_slice() else {
        return Err(format!("two periods, newest first, found {leverage}").into());
    };

    assert_eq!(february["period_start"], "2026-02-01T00:00:00Z");
    assert_eq!(february["period_end"], "2026-03-01T00:00:00Z");
    assert_eq!(february["complete"], true);
    assert_eq!(february["requests"], 1);
    assert_eq!(february["total_tokens"], 150_000);
    close(&february["list_cost_usd"], 0.4);
    close(&february["leverage"], 1.0);
    close(&february["effective_usd_per_mtok"], 0.4 * 1e6 / 150_000.0);

    assert_eq!(january["period_start"], "2026-01-01T00:00:00Z");
    assert_eq!(january["period_end"], "2026-02-01T00:00:00Z");
    assert_eq!(january["complete"], true);
    assert_eq!(january["requests"], 2);
    assert_eq!(january["total_tokens"], 300_000);
    close(&january["list_cost_usd"], 0.8);
    close(&january["leverage"], 2.0);
    close(&january["effective_usd_per_mtok"], 0.4 * 1e6 / 300_000.0);

    let one = service.consumer.get("/api/leverage?periods=1").send().await;
    one.assert_status_is_ok();
    let one = one.json().await.value().deserialize::<Value>();
    assert_eq!(one["plans"][0]["periods"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        one["plans"][0]["periods"][0]["period_start"],
        "2026-02-01T00:00:00Z"
    );

    Ok(())
}

#[tokio::test]
async fn plans_and_leverage_answer_not_modified_until_they_change() -> Result<(), Failure> {
    let service = service().await?;
    service.use_tokens(fixture()).await?;
    let plan_id = service.create_plan(&service.account_id().await?).await?;

    let mut tags = Vec::new();
    for path in ["/api/plans", "/api/leverage"] {
        let first = service.consumer.get(path).send().await;
        first.assert_status_is_ok();
        let current = tag(&first);
        assert!(current.starts_with('"'), "{path} has no ETag");

        for if_none_match in [current.clone(), format!("W/{current}"), "*".to_owned()] {
            let again = service.read_unless(path, &if_none_match).await;
            again.assert_status(StatusCode::NOT_MODIFIED);
            assert_eq!(tag(&again), current, "{path}");
            again.assert_text("").await;
        }
        let other = service.read_unless(path, "\"another\"").await;
        other.assert_status_is_ok();
        assert_eq!(tag(&other), current, "{path}");
        tags.push(current);
    }
    let [plans_tag, leverage_tag] = tags.as_slice() else {
        return Err("two tags".into());
    };

    // Usage moves the leverage and leaves the plan list as it was.
    service
        .use_tokens(vec![record("february-b", "2026-02-20T00:00:00Z")])
        .await?;
    let plans = service.read_unless("/api/plans", plans_tag).await;
    plans.assert_status(StatusCode::NOT_MODIFIED);
    let moved = service.read_unless("/api/leverage", leverage_tag).await;
    moved.assert_status_is_ok();
    let moved_tag = tag(&moved);
    assert_ne!(&moved_tag, leverage_tag);
    let moved = moved.json().await.value().deserialize::<Value>();
    assert_eq!(moved["plans"][0]["periods"][0]["requests"], 2);

    // A new price moves both.
    let edited = service
        .admin
        .patch(format!("/api/plans/{plan_id}"))
        .body_json(&json!({ "monthly_usd": 0.8 }))
        .send()
        .await;
    edited.assert_status_is_ok();
    let plans = service.read_unless("/api/plans", plans_tag).await;
    plans.assert_status_is_ok();
    assert_ne!(&tag(&plans), plans_tag);
    let plans = plans.json().await.value().deserialize::<Value>();
    close(&plans["plans"][0]["monthly_usd"], 0.8);
    let leverage = service.read_unless("/api/leverage", &moved_tag).await;
    leverage.assert_status_is_ok();
    assert_ne!(tag(&leverage), moved_tag);
    let leverage = leverage.json().await.value().deserialize::<Value>();
    close(&leverage["plans"][0]["periods"][0]["leverage"], 1.0);
    close(&leverage["plans"][0]["periods"][1]["leverage"], 1.0);

    Ok(())
}

#[tokio::test]
async fn each_admin_plan_write_reaches_the_consumer_feed() -> Result<(), Failure> {
    let service = service().await?;
    service.use_tokens(fixture()).await?;
    let account_id = service.account_id().await?;
    let cursor = service.latest().await?;

    let plan_id = service.create_plan(&account_id).await?;
    let created = service.changes(cursor, "plan.changed").await;
    assert_eq!(
        plan_changes(&created),
        [("plan.changed".to_owned(), plan_id.clone())]
    );
    let after_create = created["cursor"].as_i64().ok_or("a cursor")?;

    let edited = service
        .admin
        .patch(format!("/api/plans/{plan_id}"))
        .body_json(&json!({ "name": "Max 20x", "period_end": null }))
        .send()
        .await;
    edited.assert_status_is_ok();
    let edited = edited.json().await.value().deserialize::<Value>();
    assert_eq!(edited["name"], "Max 20x");
    assert!(edited.get("period_end").is_none(), "{edited}");
    let updated = service.changes(after_create, "").await;
    assert_eq!(
        plan_changes(&updated),
        [("plan.changed".to_owned(), plan_id.clone())]
    );
    let after_update = updated["cursor"].as_i64().ok_or("a cursor")?;

    let deleted = service
        .admin
        .delete(format!("/api/plans/{plan_id}"))
        .send()
        .await;
    deleted.assert_status(StatusCode::NO_CONTENT);
    let removed = service.changes(after_update, "plan.changed").await;
    assert_eq!(
        plan_changes(&removed),
        [("plan.changed".to_owned(), plan_id.clone())]
    );
    assert_eq!(service.plans().await["plans"], json!([]));

    let all = service.changes(cursor, "plan.changed").await;
    assert_eq!(plan_changes(&all).len(), 3, "{all}");
    let unrelated = service.changes(cursor, "price.changed,usage.updated").await;
    assert_eq!(unrelated["events"], json!([]));

    Ok(())
}

#[tokio::test]
async fn unchanged_refused_and_unknown_plan_writes_record_nothing() -> Result<(), Failure> {
    let service = service().await?;
    service.use_tokens(fixture()).await?;
    let account_id = service.account_id().await?;
    let plan_id = service.create_plan(&account_id).await?;
    let before = service.plans().await;
    let cursor = service.latest().await?;
    let unknown_plan = Id::<Plan>::from_raw(1).encode();
    let unknown_account = Id::<Account>::from_raw(1).encode();

    for (body, status) in [
        (json!({}), StatusCode::OK),
        (
            json!({
                "account_id": account_id, "name": "  Max  ", "monthly_usd": 0.4,
                "period_start": "2026-01-01T00:00:00Z", "period_end": "2026-03-01T00:00:00Z",
            }),
            StatusCode::OK,
        ),
        (json!({ "name": "   " }), StatusCode::BAD_REQUEST),
        (json!({ "monthly_usd": -1.0 }), StatusCode::BAD_REQUEST),
        (
            json!({ "period_end": "2026-01-01T00:00:00Z" }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "period_start": "not a time" }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "account_id": unknown_account }),
            StatusCode::NOT_FOUND,
        ),
        (json!({ "account_id": "not an id" }), StatusCode::NOT_FOUND),
    ] {
        let response = service
            .admin
            .patch(format!("/api/plans/{plan_id}"))
            .body_json(&body)
            .send()
            .await;
        response.assert_status(status);
    }
    for path in [
        format!("/api/plans/{unknown_plan}"),
        "/api/plans/nope".to_owned(),
    ] {
        let edited = service
            .admin
            .patch(&path)
            .body_json(&json!({ "name": "Other" }))
            .send()
            .await;
        edited.assert_status(StatusCode::NOT_FOUND);
        let deleted = service.admin.delete(&path).send().await;
        deleted.assert_status(StatusCode::NOT_FOUND);
    }
    for (body, status) in [
        (
            json!({
                "account_id": account_id, "name": " ", "monthly_usd": 1.0,
                "period_start": "2026-01-01T00:00:00Z",
            }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({
                "account_id": unknown_account, "name": "Other", "monthly_usd": 1.0,
                "period_start": "2026-01-01T00:00:00Z",
            }),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let created = service
            .admin
            .post("/api/plans")
            .body_json(&body)
            .send()
            .await;
        created.assert_status(status);
    }

    let answer = service.changes(cursor, "").await;
    assert_eq!(answer["events"], json!([]));
    assert_eq!(answer["cursor"], cursor);
    assert_eq!(service.plans().await, before);

    Ok(())
}

#[tokio::test]
async fn a_plan_write_whose_change_is_refused_leaves_the_plan_as_it_was() -> Result<(), Failure> {
    let service = service().await?;
    service.use_tokens(fixture()).await?;
    let account_id = service.account_id().await?;
    let plan_id = service.create_plan(&account_id).await?;
    let before = service.plans().await;
    let cursor = service.latest().await?;
    sqlx::query(
        "CREATE TRIGGER reject_plan_change BEFORE INSERT ON change \
         WHEN NEW.kind = 'plan.changed' BEGIN SELECT RAISE(ABORT, 'rejected change'); END",
    )
    .execute(&service.state.database.pool)
    .await?;

    let created = service
        .admin
        .post("/api/plans")
        .body_json(&json!({
            "account_id": account_id, "name": "Second", "monthly_usd": 1.0,
            "period_start": "2026-01-01T00:00:00Z",
        }))
        .send()
        .await;
    created.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
    let edited = service
        .admin
        .patch(format!("/api/plans/{plan_id}"))
        .body_json(&json!({ "name": "Renamed" }))
        .send()
        .await;
    edited.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
    let deleted = service
        .admin
        .delete(format!("/api/plans/{plan_id}"))
        .send()
        .await;
    deleted.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

    assert_eq!(service.plans().await, before);
    assert_eq!(service.latest().await?, cursor);

    Ok(())
}

#[tokio::test]
async fn the_consumer_token_cannot_write_plans() -> Result<(), Failure> {
    let service = service().await?;
    service.use_tokens(fixture()).await?;
    let account_id = service.account_id().await?;
    let plan_id = service.create_plan(&account_id).await?;
    let before = service.plans().await;
    let cursor = service.latest().await?;
    let path = format!("/api/plans/{plan_id}");

    let created = service
        .consumer
        .post("/api/plans")
        .body_json(&json!({
            "account_id": account_id, "name": "Second", "monthly_usd": 1.0,
            "period_start": "2026-01-01T00:00:00Z",
        }))
        .send()
        .await;
    created.assert_status(StatusCode::UNAUTHORIZED);
    let edited = service
        .consumer
        .patch(&path)
        .body_json(&json!({ "name": "Renamed" }))
        .send()
        .await;
    edited.assert_status(StatusCode::UNAUTHORIZED);
    let deleted = service.consumer.delete(&path).send().await;
    deleted.assert_status(StatusCode::UNAUTHORIZED);

    assert_eq!(service.plans().await, before);
    assert_eq!(service.changes(cursor, "").await["events"], json!([]));

    Ok(())
}
