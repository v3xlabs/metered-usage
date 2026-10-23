use std::sync::{Arc, Mutex};

use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::source::Source;
use poem::http::{StatusCode, header};
use poem::listener::{Acceptor, Listener, TcpListener};
use poem::test::TestClient;
use poem::web::{Data, Json};
use poem::{EndpointExt, IntoResponse, Request, Response, Route, Server, get, handler, post};
use serde_json::{Value, json};

const TEST_ACCESS_TOKEN: &str = "test-access-token-not-a-secret";
const TEST_MANAGEMENT_KEY: &str = "test-management-key-not-a-secret";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const CONSUME_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_RESET_GRANTS_URL: &str =
    "https://api.anthropic.com/api/oauth/usage?cedar_ember=1&skip_spend=1";
const CODEX_RECORDED: &str = include_str!("fixtures/codex_reset_credits.json");
const CLAUDE_RECORDED: &str = include_str!("fixtures/claude_reset_grants.json");
const COOLDOWN_RESET: &str = "reset-quota";
const EXPIRED: &str = "2020-01-01T00:00:00Z";
const SOONEST: &str = "2099-01-01T00:00:00Z";
const LATEST: &str = "2099-02-01T00:00:00Z";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Listed {
    Enabled,
    Disabled,
    Removed,
}

#[derive(Clone)]
struct Upstream(Arc<Mutex<State>>);

struct State {
    log: Vec<String>,
    calls: Vec<Value>,
    cooldown_resets: Vec<Value>,
    credits: Vec<&'static str>,
    consume_response: (u16, Value),
    consumed: bool,
    usage_fails_after_reset: bool,
    credits_fail: bool,
    credits_recorded: Option<Value>,
    claude_grants: Option<Value>,
    claude_usage_status: u16,
    codex: Listed,
}

impl Upstream {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            log: Vec::new(),
            calls: Vec::new(),
            cooldown_resets: Vec::new(),
            credits: vec![EXPIRED, LATEST, SOONEST],
            consume_response: (200, json!({ "code": "reset" })),
            consumed: false,
            usage_fails_after_reset: false,
            credits_fail: false,
            credits_recorded: None,
            claude_grants: None,
            claude_usage_status: 200,
            codex: Listed::Enabled,
        })))
    }

    fn with<T>(&self, read: impl FnOnce(&mut State) -> T) -> T {
        match self.0.lock() {
            Ok(mut state) => read(&mut state),
            Err(poisoned) => read(&mut poisoned.into_inner()),
        }
    }

    fn sent(&self, url: &str) -> Vec<Value> {
        self.with(|state| {
            state
                .calls
                .iter()
                .filter(|call| call["url"] == url)
                .cloned()
                .collect()
        })
    }

    fn listing(&self) -> Value {
        self.with(|state| {
            let mut files = vec![json!({
                "id": "claude.json", "auth_index": "a1", "name": "claude.json",
                "type": "claude", "provider": "claude", "email": "ada@example.com",
                "account_type": "oauth", "status": "active", "disabled": false,
                "unavailable": false, "cooldowns": [],
            })];
            if state.codex != Listed::Removed {
                files.push(json!({
                    "id": "codex.json", "auth_index": "a2", "name": "codex.json",
                    "type": "codex", "provider": "codex", "email": "bo@example.com",
                    "status": "active", "disabled": state.codex == Listed::Disabled,
                    "unavailable": true, "cooldowns": [],
                    "id_token": { "chatgpt_account_id": "acct-77", "plan_type": "plus" },
                }));
            }
            json!({ "files": files })
        })
    }

    fn answer(&self, call: &Value) -> (u16, Value) {
        self.with(|state| {
            let url = call["url"].as_str().unwrap_or_default().to_owned();
            state.log.push(url.clone());
            state.calls.push(call.clone());
            match url.as_str() {
                USAGE_URL if state.consumed && state.usage_fails_after_reset => {
                    (503, json!({ "error": "overloaded" }))
                }
                USAGE_URL => {
                    let used = if state.consumed { 0 } else { 100 };
                    let window = json!({
                        "used_percent": used,
                        "limit_window_seconds": 18_000,
                        "reset_at": 4_070_908_800_i64,
                    });
                    (
                        200,
                        json!({
                            "plan_type": "plus",
                            "rate_limit": {
                                "allowed": !state.consumed,
                                "limit_reached": !state.consumed,
                                "primary_window": window,
                            },
                        }),
                    )
                }
                RESET_CREDITS_URL if state.credits_fail => (500, json!({ "error": "down" })),
                RESET_CREDITS_URL => (
                    200,
                    state
                        .credits_recorded
                        .clone()
                        .unwrap_or_else(|| reset_credits(&state.credits)),
                ),
                CONSUME_URL => {
                    state.consumed = true;
                    if let Some(expires_at) = state
                        .credits
                        .iter()
                        .copied()
                        .filter(|expires_at| *expires_at != EXPIRED)
                        .min()
                    {
                        state.credits.retain(|listed| *listed != expires_at);
                    }
                    state.consume_response.clone()
                }
                CLAUDE_USAGE_URL if state.claude_usage_status != 200 => {
                    (state.claude_usage_status, json!({ "error": "overloaded" }))
                }
                CLAUDE_USAGE_URL => (
                    200,
                    json!({ "five_hour": { "utilization": 10, "resets_at": SOONEST } }),
                ),
                CLAUDE_RESET_GRANTS_URL => match &state.claude_grants {
                    Some(grants) => (200, grants.clone()),
                    None => (429, json!({ "error": "rate_limited" })),
                },
                _ => (401, json!({ "error": "unauthorized" })),
            }
        })
    }
}

fn reset_credits(available: &[&str]) -> Value {
    let mut credits = available
        .iter()
        .map(|expires_at| {
            json!({
                "id": format!("credit-{expires_at}"),
                "reset_type": "codex_rate_limits",
                "status": "available",
                "granted_at": "2019-01-01T00:00:00Z",
                "expires_at": expires_at,
            })
        })
        .collect::<Vec<_>>();
    credits.push(json!({
        "id": "spent", "reset_type": "codex_rate_limits", "status": "redeemed",
        "expires_at": LATEST,
    }));

    json!({ "available_count": available.len(), "credits": credits })
}

fn authorized(request: &Request) -> bool {
    request.header(header::AUTHORIZATION) == Some(&format!("Bearer {TEST_MANAGEMENT_KEY}"))
}

#[handler]
fn auth_files(request: &Request, Data(upstream): Data<&Upstream>) -> Response {
    if !authorized(request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(upstream.listing()).into_response()
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
    let (status, body) = upstream.answer(&call);
    Json(json!({ "status_code": status, "header": {}, "body": body.to_string() })).into_response()
}

#[handler]
fn reset_quota(
    request: &Request,
    Data(upstream): Data<&Upstream>,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    upstream.with(|state| {
        state.log.push(COOLDOWN_RESET.to_owned());
        state.cooldown_resets.push(body.clone());
    });
    Json(json!({ "status": "ok", "auth_index": body["auth_index"], "models": [] })).into_response()
}

async fn service_with_state(
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
        .at("/v0/management/reset-quota", post(reset_quota))
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

    Ok((
        TestClient::new(metered_usage::http::routes(&state))
            .default_header("X-Quota-Client", "test-ui")
            .default_header(header::AUTHORIZATION, format!("Bearer {TEST_ACCESS_TOKEN}")),
        state,
    ))
}

async fn service(
    upstream: &Upstream,
) -> Result<TestClient<impl poem::Endpoint>, Box<dyn std::error::Error>> {
    let (client, _) = service_with_state(upstream).await?;
    Ok(client)
}

async fn send(
    client: &TestClient<impl poem::Endpoint>,
    path: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let response = client.post(path).body_json(body).send().await;
    let status = response.0.status();
    (status, response.json().await.value().deserialize::<Value>())
}

async fn reset(client: &TestClient<impl poem::Endpoint>, account_id: &str) -> (StatusCode, Value) {
    send(
        client,
        "/api/quota/reset",
        &json!({ "account_id": account_id, "confirmed": true }),
    )
    .await
}

async fn refreshed(client: &TestClient<impl poem::Endpoint>) -> (String, String) {
    let (status, refreshed) = send(client, "/api/quota/refresh", &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");

    (
        account_id(&refreshed, "codex"),
        account_id(&refreshed, "claude"),
    )
}

async fn changes(client: &TestClient<impl poem::Endpoint>, query: &str) -> Value {
    let response = client.get(format!("/api/events?{query}")).send().await;
    response.assert_status_is_ok();
    response.json().await.value().deserialize::<Value>()
}

async fn listed(client: &TestClient<impl poem::Endpoint>) -> Value {
    let response = client.get("/api/quota").send().await;
    response.assert_status_is_ok();
    response.json().await.value().deserialize::<Value>()
}

fn by_provider<'a>(accounts: &'a Value, provider: &str) -> &'a Value {
    accounts["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|account| account["account"]["provider"] == provider)
        .unwrap_or(&Value::Null)
}

fn account_id(accounts: &Value, provider: &str) -> String {
    by_provider(accounts, provider)["account"]["account_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn expiries(account: &Value) -> Vec<&str> {
    account["manual_resets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|reset| reset["expires_at"].as_str())
        .collect()
}

#[tokio::test]
async fn a_hard_refresh_lists_unexpired_manual_resets_and_keeps_them_when_the_list_fails() {
    let upstream = Upstream::new();
    let (client, state) = service_with_state(&upstream).await.expect("a service");
    refreshed(&client).await;
    let calls = upstream.with(|state| state.calls.len());

    let listed = listed(&client).await;
    assert_eq!(expiries(by_provider(&listed, "codex")), [SOONEST, LATEST]);
    assert_eq!(by_provider(&listed, "claude")["manual_resets"], json!([]));
    assert_eq!(
        upstream.with(|state| state.calls.len()),
        calls,
        "listing reached a provider"
    );

    let lists = upstream.sent(RESET_CREDITS_URL);
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0]["authIndex"], "a2");
    assert_eq!(lists[0]["method"], "GET");
    assert_eq!(
        lists[0]["header"],
        json!({
            "Authorization": "Bearer $TOKEN$",
            "Content-Type": "application/json",
            "User-Agent": "codex-tui/0.149.1 (Mac OS 26.5.2; arm64) iTerm.app/3.6.11 (codex-tui; 0.149.1)",
            "Chatgpt-Account-Id": "acct-77",
            "Accept": "application/json",
            "OpenAI-Beta": "codex-1",
            "Originator": "Codex Desktop",
        })
    );
    assert!(
        upstream.sent(CONSUME_URL).is_empty(),
        "a refresh spent a reset"
    );

    upstream.with(|state| state.credits_fail = true);
    expire_actions(&state).await;
    let (status, again) = send(&client, "/api/quota/refresh", &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["failed"], 0);
    assert_eq!(expiries(by_provider(&again, "codex")), [SOONEST, LATEST]);
}

#[tokio::test]
async fn a_reset_without_confirmed_true_is_refused_and_spends_nothing() {
    let upstream = Upstream::new();
    let client = service(&upstream).await.expect("a service");
    let (codex, _) = refreshed(&client).await;

    let (status, refused) = send(
        &client,
        "/api/quota/reset",
        &json!({ "account_id": codex, "confirmed": false }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("confirmed")),
        "{refused}"
    );
    client
        .post("/api/quota/reset")
        .body_json(&json!({ "account_id": codex }))
        .send()
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    assert!(
        upstream.sent(CONSUME_URL).is_empty(),
        "an unconfirmed reset was sent"
    );
    assert_eq!(
        expiries(by_provider(&listed(&client).await, "codex")),
        [SOONEST, LATEST]
    );
}

#[tokio::test]
async fn a_confirmed_reset_spends_one_then_clears_the_cooldown_and_refreshes_the_quota() {
    let upstream = Upstream::new();
    let client = service(&upstream).await.expect("a service");
    let (codex, _) = refreshed(&client).await;
    upstream.with(|state| state.log.clear());

    let (status, reset) = reset(&client, &codex).await;

    assert_eq!(status, StatusCode::OK, "{reset}");
    assert_eq!(reset["refreshed"], 1);
    assert_eq!(reset["failed"], 0);
    let account = by_provider(&reset, "codex");
    assert_eq!(account["windows"][0]["used_fraction"], 0.0);
    assert!(account.get("hard_refresh_error").is_none(), "{account}");
    assert_eq!(expiries(account), [LATEST]);

    let consumed = upstream.sent(CONSUME_URL);
    assert_eq!(consumed.len(), 1);
    assert_eq!(consumed[0]["authIndex"], "a2");
    assert_eq!(consumed[0]["method"], "POST");
    assert_eq!(consumed[0]["header"]["Authorization"], "Bearer $TOKEN$");
    assert_eq!(consumed[0]["header"]["Chatgpt-Account-Id"], "acct-77");
    let data = serde_json::from_str::<Value>(consumed[0]["data"].as_str().expect("a body"))
        .expect("a JSON body");
    let request_id = data["redeem_request_id"].as_str().expect("a request id");
    assert_eq!(request_id.len(), 36, "{request_id}");
    assert_eq!(data.as_object().map(serde_json::Map::len), Some(1));

    let (cooldown_resets, order) = upstream.with(|state| {
        let order = state
            .log
            .iter()
            .filter(|step| [CONSUME_URL, COOLDOWN_RESET, USAGE_URL].contains(&step.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        (state.cooldown_resets.clone(), order)
    });
    assert_eq!(cooldown_resets, [json!({ "auth_index": "a2" })]);
    assert_eq!(order, [CONSUME_URL, COOLDOWN_RESET, USAGE_URL]);
}

#[tokio::test]
async fn a_spent_reset_whose_refresh_fails_answers_failed_and_is_not_offered_again() {
    let upstream = Upstream::new();
    upstream.with(|state| state.credits = vec![SOONEST]);
    let (client, state) = service_with_state(&upstream).await.expect("a service");
    let (codex, _) = refreshed(&client).await;
    upstream.with(|state| state.usage_fails_after_reset = true);
    let before = changes(&client, "").await;

    expire_actions(&state).await;
    let (status, spent) = reset(&client, &codex).await;
    assert_eq!(status, StatusCode::OK, "{spent}");
    assert_eq!(spent["refreshed"], 0);
    assert_eq!(spent["failed"], 1);
    let account = by_provider(&spent, "codex");
    assert_eq!(account["hard_refresh_error"], "the provider answered 503");
    assert_eq!(account["manual_resets"], json!([]));
    assert_eq!(account["windows"][0]["used_fraction"], 1.0);
    // The spent reset is a change of its own, committed before the provider is asked; the
    // failed refresh changes the account and leaves its window as it was.
    let after = changes(&client, &format!("cursor={}&timeout=0", before["cursor"])).await;
    let named = after["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|change| (change["kind"].clone(), change["account_id"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        [
            (json!("quota.action"), Value::Null),
            (json!("reset.updated"), json!(codex)),
            (json!("account.changed"), json!(codex)),
            (json!("quota.action"), Value::Null),
        ]
    );

    expire_actions(&state).await;
    let (status, refused) = reset(&client, &codex).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        upstream.sent(CONSUME_URL).len(),
        1,
        "a spent reset was sent again"
    );
}

#[tokio::test]
async fn an_unknown_reset_outcome_blocks_further_consumes_until_the_provider_list_is_read() {
    for answer in [
        (500_u16, json!({ "error": "boom" })),
        (200_u16, json!({ "code": "not_eligible" })),
    ] {
        let upstream = Upstream::new();
        let (client, state) = service_with_state(&upstream).await.expect("a service");
        let (codex, _) = refreshed(&client).await;
        upstream.with(|state| state.consume_response = answer);

        expire_actions(&state).await;
        let (status, failed) = reset(&client, &codex).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{failed}");
        assert_eq!(upstream.sent(CONSUME_URL).len(), 1);
        assert_eq!(
            upstream.with(|state| state.credits.clone()),
            [EXPIRED, LATEST]
        );
        assert!(expiries(by_provider(&listed(&client).await, "codex")).is_empty());

        expire_actions(&state).await;
        let (status, refused) = reset(&client, &codex).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert_eq!(
            refused["message"],
            "the account has no unexpired manual reset"
        );
        assert_eq!(upstream.sent(CONSUME_URL).len(), 1);

        let (status, synced) = send(&client, "/api/quota/sync", &json!({})).await;
        assert_eq!(status, StatusCode::OK, "{synced}");
        assert!(expiries(by_provider(&synced, "codex")).is_empty());

        upstream.with(|state| state.usage_fails_after_reset = true);
        expire_actions(&state).await;
        let (status, failed) = send(&client, "/api/quota/refresh", &json!({})).await;
        assert_eq!(status, StatusCode::OK, "{failed}");
        assert_eq!(failed["failed"], 1);
        assert!(expiries(by_provider(&failed, "codex")).is_empty());

        upstream.with(|state| {
            state.usage_fails_after_reset = false;
            state.credits_fail = true;
        });
        expire_actions(&state).await;
        refreshed(&client).await;
        expire_actions(&state).await;
        let (status, refused) = reset(&client, &codex).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert_eq!(upstream.sent(CONSUME_URL).len(), 1);

        upstream.with(|state| state.credits_fail = false);
        expire_actions(&state).await;
        refreshed(&client).await;
        assert_eq!(
            expiries(by_provider(&listed(&client).await, "codex")),
            [LATEST]
        );
        expire_actions(&state).await;
        let (status, failed) = reset(&client, &codex).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{failed}");
        expire_actions(&state).await;
        let (status, refused) = reset(&client, &codex).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert_eq!(upstream.sent(CONSUME_URL).len(), 2);
        assert!(upstream.with(|state| state.cooldown_resets.is_empty()));
    }
}

#[tokio::test]
async fn concurrent_resets_cannot_consume_two_credits_after_an_unknown_outcome() {
    let upstream = Upstream::new();
    let client = service(&upstream).await.expect("a service");
    let (codex, _) = refreshed(&client).await;
    upstream.with(|state| {
        state.consume_response = (500, json!({ "error": "response lost after consuming" }));
    });

    let ((first_status, _), (second_status, _)) =
        tokio::join!(reset(&client, &codex), reset(&client, &codex));
    assert_eq!(first_status, StatusCode::BAD_GATEWAY);
    assert_eq!(second_status, first_status);
    assert_eq!(upstream.sent(CONSUME_URL).len(), 1);
    assert_eq!(
        upstream.with(|state| state.credits.clone()),
        [EXPIRED, LATEST]
    );
    assert!(expiries(by_provider(&listed(&client).await, "codex")).is_empty());
}

/// The recorded answers, moved into the future so they stay unexpired.
fn recorded() -> Result<(Value, Value), serde_json::Error> {
    let mut codex = serde_json::from_str::<Value>(CODEX_RECORDED)?;
    codex["credits"][0]["expires_at"] = json!(SOONEST);
    codex["credits"][1]["expires_at"] = json!(LATEST);
    let mut claude = serde_json::from_str::<Value>(CLAUDE_RECORDED)?;
    claude["cedar_ember"]["grants"][0]["ends_at"] = json!(SOONEST);

    Ok((codex, claude))
}

#[tokio::test]
async fn recorded_reset_lists_of_both_providers_list_as_grants() {
    let upstream = Upstream::new();
    let (codex_credits, claude_grants) = recorded().expect("the fixtures are JSON");
    upstream.with(|state| {
        state.credits_recorded = Some(codex_credits);
        state.claude_grants = Some(claude_grants.clone());
    });
    let (client, state) = service_with_state(&upstream).await.expect("a service");
    let (_, claude) = refreshed(&client).await;

    let listed = listed(&client).await;
    assert_eq!(
        by_provider(&listed, "codex")["manual_resets"],
        json!([
            { "remaining": 1, "expires_at": SOONEST, "spendable": true },
            { "remaining": 1, "expires_at": LATEST, "spendable": true },
        ])
    );
    let claude_resets = json!([
        {
            "label": "Claude Opus 5.5 launch: one usage-limit reset for Pro and Max",
            "remaining": 1,
            "total": 1,
            "expires_at": SOONEST,
            "spendable": false,
        },
        { "label": "Max loyalty resets", "remaining": 2, "total": 3, "spendable": false },
    ]);
    assert_eq!(
        by_provider(&listed, "claude")["manual_resets"],
        claude_resets
    );

    let reads = upstream.sent(CLAUDE_RESET_GRANTS_URL);
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0]["authIndex"], "a1");
    assert_eq!(reads[0]["method"], "GET");
    assert_eq!(
        reads[0]["header"],
        json!({
            "User-Agent": "claude-cli/2.1.280 (external, cli)",
            "Authorization": "Bearer $TOKEN$",
            "Content-Type": "application/json",
            "anthropic-beta": "oauth-2025-04-20",
        })
    );

    upstream.with(|state| state.claude_grants = None);
    expire_actions(&state).await;
    let (status, again) = send(&client, "/api/quota/refresh", &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["failed"], 0);
    let account = by_provider(&again, "claude");
    assert_eq!(account["manual_resets"], claude_resets);
    assert_eq!(account["windows"][0]["used_fraction"], 0.1);

    let calls = upstream.with(|state| state.calls.len());
    expire_actions(&state).await;
    let (status, refused) = reset(&client, &claude).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        upstream.with(|state| state.calls.len()),
        calls,
        "a Claude reset reached the provider"
    );

    upstream.with(|state| state.claude_usage_status = 503);
    let reads = upstream.sent(CLAUDE_RESET_GRANTS_URL).len();
    expire_actions(&state).await;
    let (status, failed) = send(&client, "/api/quota/refresh", &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    assert_eq!(failed["failed"], 1);
    assert_eq!(
        upstream.sent(CLAUDE_RESET_GRANTS_URL).len(),
        reads,
        "the grants were read although the usage read failed"
    );
    upstream.with(|state| state.claude_usage_status = 200);

    let mut used_up = claude_grants;
    for grant in used_up["cedar_ember"]["grants"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        grant["resets_left"] = json!(0);
    }
    upstream.with(|state| state.claude_grants = Some(used_up));
    expire_actions(&state).await;
    let (status, cleared) = send(&client, "/api/quota/refresh", &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(by_provider(&cleared, "claude")["manual_resets"], json!([]));
}

#[tokio::test]
async fn a_reset_that_cannot_be_spent_is_refused_before_the_provider_is_asked() {
    let upstream = Upstream::new();
    let (client, state) = service_with_state(&upstream).await.expect("a service");

    let (status, synced) = send(&client, "/api/quota/sync", &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    expire_actions(&state).await;
    let (status, refused) = reset(&client, &account_id(&synced, "codex")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    upstream.with(|state| state.credits = vec![EXPIRED]);
    let (codex, claude) = refreshed(&client).await;
    expire_actions(&state).await;
    let (status, refused) = reset(&client, &codex).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an expired reset: {refused}"
    );

    upstream.with(|state| state.credits = vec![SOONEST]);
    expire_actions(&state).await;
    refreshed(&client).await;
    expire_actions(&state).await;
    let (status, refused) = reset(&client, &claude).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "no manual resets: {refused}"
    );
    expire_actions(&state).await;
    let (status, refused) = reset(&client, "not-an-id").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");

    upstream.with(|state| state.codex = Listed::Disabled);
    expire_actions(&state).await;
    let (status, refused) = reset(&client, &codex).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a disabled credential: {refused}"
    );
    assert_eq!(
        by_provider(&listed(&client).await, "codex")["manual_resets"][0]["spendable"],
        true,
        "a disabled Codex credential lost its reset action"
    );
    upstream.with(|state| state.codex = Listed::Removed);
    expire_actions(&state).await;
    let (status, refused) = reset(&client, &codex).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a removed credential: {refused}"
    );

    assert!(
        upstream.sent(CONSUME_URL).is_empty(),
        "a refused reset was sent"
    );
}

async fn expire_actions(state: &AppState) {
    let result = sqlx::query("UPDATE quota_action SET finished_at = '2000-01-01T00:00:00Z' WHERE finished_at IS NOT NULL")
        .execute(&state.database.pool).await;
    assert!(result.is_ok(), "expire saved actions: {result:?}");
}
