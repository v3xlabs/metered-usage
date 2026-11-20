use std::sync::Arc;

use metered_usage::app::AppState;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::{Credentials, Token};
use metered_usage::source::Source;
use poem::http::{StatusCode, header};
use poem::listener::{Acceptor, Listener, TcpListener};
use poem::test::TestClient;
use poem::web::Json;
use poem::{IntoResponse, Request, Response, Route, Server, get, handler, post};
use serde_json::{Value, json};

const TOKEN: &str = "correct horse battery staple";
const MANAGEMENT_KEY: &str = "management-key-4f1c9a";
const DAILY: &str = include_str!("fixtures/antigravity_quota_daily.json");
const PRODUCTION: &str = include_str!("fixtures/antigravity_quota_production.json");

type Failure = Box<dyn std::error::Error>;

#[handler]
fn auth_files(request: &Request) -> Response {
    if request.header(header::AUTHORIZATION) != Some(&format!("Bearer {MANAGEMENT_KEY}")) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({
        "observed_at": "2026-11-20T10:00:00Z",
        "files": [{
            "auth_index": "ag-1", "name": "ag-1.json", "type": "antigravity",
            "provider": "antigravity", "label": "", "status": "active", "disabled": false,
            "unavailable": false, "project_id": "project-1", "cooldowns": [],
        }],
    }))
    .into_response()
}

/// The daily host answers with the Claude and GPT group alone and the production host with all
/// three groups, as CLIProxy's management center reports for current accounts.
#[handler]
fn api_call(Json(call): Json<Value>) -> Response {
    let (status, body) = match call["url"].as_str().unwrap_or_default() {
        "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary" => {
            (200, DAILY)
        }
        "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary" => {
            (200, PRODUCTION)
        }
        _ => (404, "{}"),
    };
    Json(json!({ "status_code": status, "header": {}, "body": body })).into_response()
}

async fn service() -> Result<TestClient<impl poem::Endpoint>, Failure> {
    let acceptor = TcpListener::bind("127.0.0.1:0").into_acceptor().await?;
    let address = acceptor
        .local_addr()
        .first()
        .and_then(|address| address.as_socket_addr().copied())
        .ok_or("the listener has no socket address")?;
    let app = Route::new()
        .at("/v0/management/auth-files", get(auth_files))
        .at("/v0/management/api-call", post(api_call));
    tokio::spawn(Server::new_with_acceptor(acceptor).run(app));

    let config = Config::parse(
        &format!(
            "[[source]]\nkey = \"work\"\nkind = \"cliproxy\"\nbase_url = \"http://{address}\"\nkey_env = \"CLIPROXY_KEY\"\n"
        ),
        |variable| (variable == "CLIPROXY_KEY").then(|| MANAGEMENT_KEY.to_owned()),
    )?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let credentials = Credentials {
        token: Token::try_from(TOKEN.to_owned())?,
        read_token: None,
        consumer_token: None,
    };
    let state = Arc::new(AppState::new(database, credentials, config)?);

    Ok(TestClient::new(metered_usage::http::routes(&state))
        .default_header("X-Quota-Client", "test-ui")
        .default_header(header::AUTHORIZATION, format!("Bearer {TOKEN}")))
}

#[tokio::test]
async fn every_group_any_host_reports_is_stored_with_a_short_label() -> Result<(), Failure> {
    let client = service().await?;
    let response = client
        .post("/api/quota/refresh")
        .body_json(&json!({}))
        .send()
        .await;
    response.assert_status_is_ok();
    let refreshed = response.json().await.value().deserialize::<Value>();

    let account = &refreshed["accounts"][0];
    assert!(account.get("hard_refresh_error").is_none(), "{account}");
    let windows = account["windows"].as_array().expect("windows");
    let stored = windows
        .iter()
        .map(|window| {
            (
                window["window_key"].as_str().unwrap_or_default(),
                window["label"].as_str().unwrap_or_default(),
                window["used_fraction"].as_f64().unwrap_or(f64::NAN),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stored,
        [
            ("3p-5h", "Claude and GPT, 5-hour", 0.0),
            ("3p-weekly", "Claude and GPT, Weekly", 0.0),
            ("gemini-5h", "Gemini, 5-hour", 0.75),
            ("gemini-weekly", "Gemini, Weekly", 0.5),
            ("gemini-argon-wtu-weekly", "Gemini 4 Argon, Weekly", 0.0),
        ]
    );

    Ok(())
}
