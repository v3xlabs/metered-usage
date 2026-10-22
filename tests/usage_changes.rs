use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use metered_usage::account::Account;
use metered_usage::app::AppState;
use metered_usage::change::{Change, RETENTION};
use metered_usage::config::Config;
use metered_usage::database::{Database, DatabaseError};
use metered_usage::http::auth::{Credentials, Token};
use metered_usage::plan::{NewPlan, Plan};
use metered_usage::price::{ModelPrice, NewPrice, Origin, Rates};
use metered_usage::source::Source;
use metered_usage::usage::cliproxy;
use metered_usage::usage::ingest::{IngestReport, ingest};
use poem::Endpoint;
use poem::http::{StatusCode, header};
use poem::test::TestClient;
use serde_json::{Value, json};
use tokio::time::Instant;

const CONSUMER: &str = "test-consumer";

type Failure = Box<dyn std::error::Error>;

async fn service() -> Result<(Arc<AppState>, TestClient<impl Endpoint>), Failure> {
    let config = Config::parse(
        "[[source]]\nkey = 'local'\nkind = 'cliproxy'\nbase_url = 'http://127.0.0.1:1'\nkey_env = 'PROXY_KEY'",
        |_| Some("test-key".to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let state = Arc::new(AppState::new(
        database,
        Credentials {
            token: Token::try_from("test-admin".to_owned())?,
            read_token: None,
            consumer_token: Some(Token::try_from(CONSUMER.to_owned())?),
        },
        config,
    )?);
    let client = TestClient::new(metered_usage::http::routes(&state))
        .default_header(header::AUTHORIZATION, format!("Bearer {CONSUMER}"));
    Ok((state, client))
}

fn record(id: &str, source: &str) -> Value {
    json!({
        "timestamp": "2026-01-15T10:00:00Z", "provider": "openai", "model": "consumer-model",
        "endpoint": "POST /v1/chat/completions", "request_id": id,
        "auth_type": "api-key", "source": source,
        "tokens": { "input_tokens": 100, "output_tokens": 50, "total_tokens": 150 },
        "fail": { "status_code": 200 }
    })
}

async fn batch(state: &AppState, records: Vec<Value>) -> Result<IngestReport, DatabaseError> {
    let source = Source::list(&state.database).await?.remove(0);
    ingest(
        state,
        source.id,
        records.into_iter().map(cliproxy::parse).collect(),
    )
    .await
}

async fn changes(client: &TestClient<impl Endpoint>, cursor: i64, kinds: &str) -> Value {
    let response = client
        .get(format!(
            "/api/events?cursor={cursor}&kinds={kinds}&timeout=0"
        ))
        .send()
        .await;
    response.assert_status_is_ok();
    response.json().await.value().deserialize()
}

async fn price(state: &AppState, input_rate: f64) -> Result<(), Failure> {
    ModelPrice::insert(
        &state.database,
        &NewPrice {
            provider: None,
            model: "consumer-model".to_owned(),
            service_tier: None,
            min_input_tokens: 0,
            effective_from: "2026-01-01T00:00:00Z".parse()?,
            rates: Rates {
                input_usd_per_mtok: input_rate,
                cached_input_usd_per_mtok: 0.0,
                cache_write_usd_per_mtok: 0.0,
                output_usd_per_mtok: 4.0,
            },
        },
        Origin::Manual,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn a_batch_wakes_a_consumer_with_one_usage_invalidation() {
    let (state, client) = service().await.unwrap();
    let cursor = Change::latest(&state.database).await.unwrap();
    let records = vec![
        record("one", "sk-account-one"),
        record("two", "sk-account-one"),
        json!({}),
    ];
    let waiting = async {
        let response = client
            .get(format!(
                "/api/events?cursor={cursor}&kinds=usage.updated&timeout=5"
            ))
            .send()
            .await;
        response.assert_status_is_ok();
        response.json().await.value().deserialize::<Value>()
    };
    let ingesting = async {
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.database.waiting_for_changes() == 0 && Instant::now() < deadline {
            tokio::task::yield_now().await;
        }
        assert_eq!(state.database.waiting_for_changes(), 1);
        batch(&state, records.clone()).await.unwrap()
    };
    let (answer, report) = tokio::join!(waiting, ingesting);
    assert_eq!(
        report,
        IngestReport {
            accepted: 2,
            duplicates: 0,
            rejected: 1
        }
    );
    let events = answer["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "usage.updated");
    assert_eq!(
        events[0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["cursor", "kind", "recorded_at"]
    );
    let after = answer["cursor"].as_i64().unwrap();
    assert!(after > cursor);

    assert_eq!(
        batch(&state, records).await.unwrap(),
        IngestReport {
            accepted: 0,
            duplicates: 2,
            rejected: 1
        }
    );
    assert_eq!(
        batch(&state, vec![]).await.unwrap(),
        IngestReport::default()
    );
    let unchanged = changes(&client, after, "usage.updated").await;
    assert_eq!(unchanged["events"], json!([]));
    assert_eq!(unchanged["cursor"], after);
}

#[tokio::test]
async fn fills_and_reprices_invalidate_usage_costs_once_per_transaction() {
    let (state, client) = service().await.unwrap();
    batch(
        &state,
        vec![
            record("one", "sk-account-one"),
            record("two", "sk-account-one"),
        ],
    )
    .await
    .unwrap();
    let before = Change::latest(&state.database).await.unwrap();
    assert_eq!(ModelPrice::fill(&state).await.unwrap(), 0);
    assert_eq!(Change::latest(&state.database).await.unwrap(), before);
    price(&state, 2.0).await.unwrap();
    let cursor = Change::latest(&state.database).await.unwrap();
    assert_eq!(ModelPrice::fill(&state).await.unwrap(), 2);
    let filled = changes(&client, cursor, "usage.updated").await;
    assert_eq!(filled["events"].as_array().unwrap().len(), 1);
    assert_eq!(filled["events"][0]["kind"], "usage.updated");
    let after_fill = filled["cursor"].as_i64().unwrap();
    assert_eq!(ModelPrice::fill(&state).await.unwrap(), 0);
    assert_eq!(Change::latest(&state.database).await.unwrap(), after_fill);

    price(&state, 3.0).await.unwrap();
    let cursor = Change::latest(&state.database).await.unwrap();
    assert_eq!(ModelPrice::reprice(&state, None, None).await.unwrap(), 2);
    let repriced = changes(&client, cursor, "usage.updated").await;
    assert_eq!(repriced["events"].as_array().unwrap().len(), 1);
    assert_eq!(repriced["events"][0]["kind"], "usage.updated");
    let cost: f64 = sqlx::query_scalar("SELECT SUM(list_cost_usd) FROM usage_event")
        .fetch_one(&state.database.pool)
        .await
        .unwrap();
    assert!((cost - 0.001).abs() < 1e-12);
}

#[tokio::test]
async fn an_account_merge_invalidates_account_grouping_and_moved_plans() {
    let (state, client) = service().await.unwrap();
    let mut second = record("two", "sk-account-two");
    second["provider"] = json!("anthropic");
    batch(&state, vec![record("one", "sk-account-one"), second])
        .await
        .unwrap();
    let accounts = Account::list(&state.database).await.unwrap();
    let from = accounts[0].summary.id;
    let into = accounts[1].summary.id;
    let plan = Plan::create(
        &state.database,
        &NewPlan {
            account_id: from,
            name: "Work".to_owned(),
            monthly_usd: 20.0,
            period_start: "2026-01-01T00:00:00Z".parse().unwrap(),
            period_end: None,
        },
    )
    .await
    .unwrap();
    let cursor = Change::latest(&state.database).await.unwrap();
    Account::merge(&state.database, from, into, false)
        .await
        .unwrap();
    let answer = changes(&client, cursor, "usage.updated,plan.changed").await;
    assert_eq!(answer["events"].as_array().unwrap().len(), 2);
    assert_eq!(answer["events"][0]["kind"], "usage.updated");
    assert_eq!(answer["events"][1]["kind"], "plan.changed");
    assert_eq!(answer["events"][1]["plan_id"], plan.id.encode());
    assert_eq!(
        Plan::load(&state.database, plan.id)
            .await
            .unwrap()
            .unwrap()
            .account
            .id,
        into
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_event WHERE account_id = ?")
        .bind(into)
        .fetch_one(&state.database.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn usage_changes_share_the_feed_retention_and_ingest_rollback() {
    let (state, client) = service().await.unwrap();
    let source = Source::list(&state.database).await.unwrap().remove(0);
    let cursor = Change::latest(&state.database).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_usage_change BEFORE INSERT ON change WHEN NEW.kind = 'usage.updated' BEGIN SELECT RAISE(ABORT, 'rejected change'); END")
        .execute(&state.database.pool).await.unwrap();
    assert!(
        ingest(
            &state,
            source.id,
            vec![cliproxy::parse(record("one", "sk-account-one"))]
        )
        .await
        .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_event")
        .fetch_one(&state.database.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(Change::latest(&state.database).await.unwrap(), cursor);
    sqlx::query("DROP TRIGGER reject_usage_change")
        .execute(&state.database.pool)
        .await
        .unwrap();
    batch(&state, vec![record("one", "sk-account-one")])
        .await
        .unwrap();
    let latest = Change::latest(&state.database).await.unwrap();
    Change::prune(
        &state.database,
        Timestamp::now() + RETENTION + SignedDuration::from_secs(1),
    )
    .await
    .unwrap();
    let response = client
        .get(format!(
            "/api/events?cursor={cursor}&kinds=usage.updated&timeout=0"
        ))
        .send()
        .await;
    response.assert_status(StatusCode::GONE);
    let answer = response.json().await.value().deserialize::<Value>();
    assert_eq!(answer["latest_cursor"], latest);
    assert_eq!(answer["oldest_cursor"], latest);
}
