use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use metered_usage::change::Change;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{batch, changes, record, service};

#[tokio::test]
async fn a_consumer_reads_every_accounts_buckets_and_follows_narrow_arrivals() {
    let (state, client) = service().await.unwrap();
    let start = Change::latest(&state.database).await.unwrap();
    // Establish three accounts, including one with no recent requests. Their requests
    // are months old, so none of them records an arrival.
    batch(
        &state,
        vec![
            account_record("old-one", "sk-account-one"),
            account_record("old-two", "sk-account-two"),
            account_record("old-idle", "sk-account-idle"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        changes(&client, start, "request.arrived").await["events"],
        json!([])
    );
    let listed = client.get("/api/accounts").send().await;
    listed.assert_status_is_ok();
    let listed = listed.json().await.value().deserialize::<Value>();
    let ids = listed["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|account| account["account_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 3);
    let initial = client.get("/api/events").send().await;
    initial.assert_status_is_ok();
    let initial = initial.json().await.value().deserialize::<Value>();
    assert_eq!(initial["events"], json!([]));
    let cursor = initial["cursor"].as_i64().unwrap();
    let now = Timestamp::now();
    let mut success = account_record("new-one", "sk-account-one");
    success["timestamp"] = json!(now.to_string());
    let mut failure = account_record("new-two", "sk-account-two");
    failure["timestamp"] = json!(now.to_string());
    failure["failed"] = json!(true);
    failure["fail"] = json!({ "status_code": 500, "body": "private upstream error" });
    let records = vec![success, failure, json!({})];
    let waiting = async {
        let response = client
            .get(format!(
                "/api/events?cursor={cursor}&kinds=request.arrived&timeout=5"
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
    assert_eq!(report.accepted, 2);
    assert_eq!(report.rejected, 1);
    let arrivals = answer["events"].as_array().unwrap();
    assert_eq!(arrivals.len(), 2);
    assert_eq!(arrivals[0]["failed"], false);
    assert_eq!(arrivals[1]["failed"], true);
    assert_ne!(arrivals[0]["account_id"], arrivals[1]["account_id"]);
    let previous = assert_arrivals(arrivals, &ids, cursor).unwrap();
    let replay = changes(&client, cursor, "request.arrived").await;
    assert_eq!(replay, answer);
    let after = answer["cursor"].as_i64().unwrap();
    assert!(after >= previous);
    // A read that names no kinds leaves arrivals out.
    let default = changes(&client, cursor, "").await;
    let kinds = default["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"usage.updated"), "{default}");
    assert!(!kinds.contains(&"request.arrived"), "{default}");
    let health = client
        .get(format!(
            "/api/analytics/health?window_minutes=10&bucket_minutes=5&account_id={}",
            ids.join(",")
        ))
        .send()
        .await;
    health.assert_status_is_ok();
    let health = health.json().await.value().deserialize::<Value>();
    assert_health(&health, &ids, arrivals).unwrap();
    let duplicate = batch(&state, records).await.unwrap();
    assert_eq!(duplicate.accepted, 0);
    assert_eq!(duplicate.duplicates, 2);
    let unchanged = changes(&client, after, "request.arrived").await;
    assert_eq!(unchanged["events"], json!([]));
    assert_eq!(unchanged["cursor"], after);
}

#[tokio::test]
async fn arrival_pages_resume_without_skipping_an_accepted_request() {
    let (state, client) = service().await.unwrap();
    let cursor = Change::latest(&state.database).await.unwrap();
    let records = (0..=metered_usage::change::PAGE)
        .map(|index| {
            let mut record = record(&format!("request-{index}"), "sk-one");
            record["timestamp"] = json!(Timestamp::now().to_string());
            record
        })
        .collect();
    batch(&state, records).await.unwrap();
    let first = changes(&client, cursor, "request.arrived").await;
    let events = first["events"].as_array().unwrap();
    assert_eq!(
        i64::try_from(events.len()).unwrap(),
        metered_usage::change::PAGE
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0]["cursor"].as_i64() < pair[1]["cursor"].as_i64())
    );
    let next = first["cursor"].as_i64().unwrap();
    assert_eq!(events.last().unwrap()["cursor"], next);
    let second = changes(&client, next, "request.arrived").await;
    let rest = second["events"].as_array().unwrap();
    assert_eq!(rest.len(), 1);
    assert!(rest[0]["cursor"].as_i64().unwrap() > next);
    let end = second["cursor"].as_i64().unwrap();
    assert!(end >= rest[0]["cursor"].as_i64().unwrap());
    assert_eq!(
        changes(&client, end, "request.arrived").await["events"],
        json!([])
    );
}

#[tokio::test]
async fn a_new_account_precedes_its_arrivals_and_old_requests_record_none() {
    let (state, client) = service().await.unwrap();
    let cursor = Change::latest(&state.database).await.unwrap();
    let now = Timestamp::now();
    let day = SignedDuration::from_hours(24);
    let minute = SignedDuration::from_mins(1);
    let mut recent = account_record("recent", "sk-account-new");
    recent["timestamp"] = json!((now - day + minute).to_string());
    let mut old = account_record("old", "sk-account-new");
    old["timestamp"] = json!((now - day - minute).to_string());
    let report = batch(&state, vec![old, recent]).await.unwrap();
    assert_eq!(report.accepted, 2);

    let answer = changes(&client, cursor, "account.changed,request.arrived").await;
    let events = answer["events"].as_array().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["account.changed", "request.arrived"],
        "{answer}"
    );
    assert_eq!(events[0]["account_id"], events[1]["account_id"]);
}

fn account_record(id: &str, account: &str) -> Value {
    let mut record = record(id, account);
    record["auth_index"] = json!(account);
    record
}

fn assert_arrivals(arrivals: &[Value], ids: &[String], cursor: i64) -> Result<i64, super::Failure> {
    let mut previous = cursor;
    for arrival in arrivals {
        // A consumer must never receive any other per-request data.
        assert_eq!(
            arrival
                .as_object()
                .ok_or("an arrival object")?
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["account_id", "cursor", "failed", "kind", "recorded_at"]
        );
        assert_eq!(arrival["kind"], "request.arrived");
        arrival["recorded_at"]
            .as_str()
            .ok_or("a recorded_at string")?
            .parse::<Timestamp>()?;
        let next = arrival["cursor"].as_i64().ok_or("an arrival cursor")?;
        assert!(next > previous);
        previous = next;
        assert!(
            ids.contains(
                &arrival["account_id"]
                    .as_str()
                    .ok_or("an account id")?
                    .to_owned()
            )
        );
    }
    Ok(previous)
}

fn assert_health(health: &Value, ids: &[String], arrivals: &[Value]) -> Result<(), super::Failure> {
    assert_eq!(
        health
            .as_object()
            .ok_or("a health object")?
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["accounts", "bucket_minutes", "buckets_start"]
    );
    assert_eq!(health["bucket_minutes"], 5);
    let start = health["buckets_start"]
        .as_str()
        .ok_or("a buckets_start string")?
        .parse::<Timestamp>()?;
    let strips = health["accounts"].as_array().ok_or("account strips")?;
    assert_eq!(strips.len(), ids.len());
    let mut expected = ids.iter().map(String::as_str).collect::<Vec<_>>();
    expected.sort_unstable();
    let actual = strips
        .iter()
        .map(|strip| strip["account_id"].as_str().ok_or("an account id"))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(actual, expected);
    for strip in strips {
        assert_eq!(
            strip
                .as_object()
                .ok_or("an account strip")?
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["account_id", "buckets"]
        );
        assert!(
            ids.contains(
                &strip["account_id"]
                    .as_str()
                    .ok_or("an account id")?
                    .to_owned()
            )
        );
        let arrival = arrivals
            .iter()
            .find(|arrival| arrival["account_id"] == strip["account_id"]);
        let buckets = strip["buckets"].as_array().ok_or("buckets")?;
        assert_eq!(buckets.len(), 2);
        let mut requests = 0;
        let mut failures = 0;
        for (index, bucket) in buckets.iter().enumerate() {
            assert_eq!(
                bucket
                    .as_object()
                    .ok_or("a bucket object")?
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["failures", "requests", "start"]
            );
            assert_eq!(
                bucket["start"]
                    .as_str()
                    .ok_or("a bucket start")?
                    .parse::<Timestamp>()?,
                start + SignedDuration::from_mins(i64::try_from(index)? * 5)
            );
            requests += bucket["requests"].as_i64().ok_or("a request count")?;
            failures += bucket["failures"].as_i64().ok_or("a failure count")?;
        }
        assert_eq!(requests, i64::from(arrival.is_some()));
        assert_eq!(
            failures,
            i64::from(arrival.is_some_and(|event| event["failed"] == true))
        );
    }
    Ok(())
}
