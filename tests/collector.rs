use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use metered_usage::app::AppState;
use metered_usage::collector;
use metered_usage::config::Config;
use metered_usage::database::Database;
use metered_usage::http::auth::Token;
use metered_usage::source::Source;
use poem::http::StatusCode;
use poem::listener::{Acceptor, Listener};
use poem::web::{Data, Json, Query};
use poem::{EndpointExt, Route, Server, handler};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, watch};

const ANTIGRAVITY: &str = include_str!("fixtures/antigravity.json");
const TOKEN: &str = "correct horse battery staple";
const MANAGEMENT_KEY: &str = "management-key-4f1c9a";
const MASTER_KEY: &str = "sk-master-7d2e0b";
const KEY_HASH: &str = "88dc28d0f030c55ed4ab77ed8faf098196cb1c05df778539800c9f1243fe6b4b";
const ROWS: i64 = 503;
const SPEND: f64 = 0.5;

type Failure = Box<dyn std::error::Error + Send + Sync>;

/// A gateway's usage queue as CLIProxyAPI keeps it: a record goes to every subscriber
/// when there is one, and is buffered for a pop only when there is none.
#[derive(Default)]
struct Queue {
    buffered: VecDeque<Vec<u8>>,
    subscribers: Vec<mpsc::UnboundedSender<Vec<u8>>>,
    /// Enqueued the moment a pop finds the buffer empty: a request that finishes after the
    /// drain's last pop reaches the collector only through a subscription already open.
    during_drain: Option<Vec<u8>>,
}

#[derive(Clone)]
struct Proxy {
    queue: Arc<Mutex<Queue>>,
    generation: Arc<watch::Sender<u64>>,
    port: u16,
}

#[derive(Deserialize)]
struct SpendQuery {
    start_date: String,
    page: usize,
    page_size: usize,
}

#[derive(Default)]
struct SpendLog {
    rows: Vec<Value>,
}

impl Queue {
    fn enqueue(&mut self, payload: Vec<u8>) {
        self.subscribers
            .retain(|subscriber| subscriber.send(payload.clone()).is_ok());
        if self.subscribers.is_empty() {
            self.buffered.push_back(payload);
        }
    }
}

impl Proxy {
    async fn start() -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let proxy = Self {
            queue: Arc::default(),
            generation: Arc::new(watch::channel(0).0),
            port: listener.local_addr()?.port(),
        };
        let accepting = proxy.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(accepting.clone().serve(stream));
            }
        });
        Ok(proxy)
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn enqueue(&self, payload: &[u8]) {
        self.queue().enqueue(payload.to_vec());
    }

    fn subscribers(&self) -> usize {
        let mut queue = self.queue();
        queue
            .subscribers
            .retain(|subscriber| !subscriber.is_closed());
        queue.subscribers.len()
    }

    /// Drops every open connection and buffers `payloads` before anyone can resubscribe.
    fn restart_with(&self, payloads: &[Vec<u8>]) {
        {
            let mut queue = self.queue();
            queue.subscribers.clear();
            queue.buffered.extend(payloads.iter().cloned());
        }
        self.generation.send_modify(|generation| *generation += 1);
    }

    async fn serve(self, stream: tokio::net::TcpStream) {
        self.converse(stream).await.ok();
    }

    async fn converse(self, stream: tokio::net::TcpStream) -> std::io::Result<()> {
        let mut restarted = self.generation.subscribe();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut authenticated = false;
        loop {
            let command = tokio::select! {
                command = read_command(&mut reader) => command,
                _ = restarted.changed() => return Ok(()),
            };
            let Some(arguments) = command else {
                return Ok(());
            };
            let name = arguments[0].to_ascii_uppercase();
            if name == b"AUTH" {
                authenticated =
                    arguments.get(1).map(Vec::as_slice) == Some(MANAGEMENT_KEY.as_bytes());
                let reply: &[u8] = if authenticated {
                    b"+OK\r\n"
                } else {
                    b"-ERR invalid management key\r\n"
                };
                write.write_all(reply).await?;
            } else if !authenticated {
                write
                    .write_all(b"-NOAUTH Authentication required.\r\n")
                    .await?;
            } else if name == b"SUBSCRIBE" {
                let (sender, mut messages) = mpsc::unbounded_channel();
                sender.send(br#"{"support_refresh":true}"#.to_vec()).ok();
                self.queue().subscribers.push(sender);
                write
                    .write_all(b"*3\r\n$9\r\nsubscribe\r\n$5\r\nusage\r\n:1\r\n")
                    .await?;
                loop {
                    tokio::select! {
                        message = messages.recv() => {
                            let Some(payload) = message else { return Ok(()) };
                            let mut frame = b"*3\r\n$7\r\nmessage\r\n$5\r\nusage\r\n".to_vec();
                            frame.extend(bulk(&payload));
                            write.write_all(&frame).await?;
                        }
                        _ = restarted.changed() => return Ok(()),
                    }
                }
            } else if name == b"LPOP" {
                let count = arguments
                    .get(2)
                    .and_then(|count| std::str::from_utf8(count).ok())
                    .and_then(|count| count.parse::<usize>().ok())
                    .unwrap_or(1);
                let popped = {
                    let mut queue = self.queue();
                    let count = count.min(queue.buffered.len());
                    let popped = queue.buffered.drain(..count).collect::<Vec<_>>();
                    if popped.is_empty()
                        && let Some(payload) = queue.during_drain.take()
                    {
                        queue.enqueue(payload);
                    }
                    popped
                };
                reply_array(&mut write, &popped).await?;
            } else {
                write.write_all(b"-ERR unknown command\r\n").await?;
            }
        }
    }
}

async fn read_command(reader: &mut BufReader<OwnedReadHalf>) -> Option<Vec<Vec<u8>>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let count = line.trim_end().strip_prefix('*')?.parse::<usize>().ok()?;
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        line.clear();
        reader.read_line(&mut line).await.ok()?;
        let length = line.trim_end().strip_prefix('$')?.parse::<usize>().ok()?;
        let mut argument = vec![0; length + 2];
        reader.read_exact(&mut argument).await.ok()?;
        argument.truncate(length);
        arguments.push(argument);
    }
    Some(arguments)
}

fn bulk(payload: &[u8]) -> Vec<u8> {
    let mut frame = format!("${}\r\n", payload.len()).into_bytes();
    frame.extend_from_slice(payload);
    frame.extend_from_slice(b"\r\n");
    frame
}

async fn reply_array(write: &mut OwnedWriteHalf, items: &[Vec<u8>]) -> std::io::Result<()> {
    let mut frame = format!("*{}\r\n", items.len()).into_bytes();
    for item in items {
        frame.extend(bulk(item));
    }
    write.write_all(&frame).await
}

fn record(request_id: &str) -> Result<Vec<u8>, Failure> {
    let mut record = serde_json::from_str::<Value>(ANTIGRAVITY)?;
    record["auth_index"] = json!("antigravity-1");
    record["request_id"] = json!(request_id);
    Ok(serde_json::to_vec(&record)?)
}

async fn state(kind: &str, base_url: &str, secret: &str) -> Result<Arc<AppState>, Failure> {
    let text = format!(
        "[[source]]\nkey = \"gateway\"\nkind = \"{kind}\"\nbase_url = \"{base_url}\"\nkey_env = \"GATEWAY_KEY\"\n"
    );
    let config = Config::parse(&text, |_| Some(secret.to_owned()))?;
    let database = Database::open("sqlite::memory:", 0).await?;
    Source::sync(&database, &config).await?;
    let token = Token::try_from(TOKEN.to_owned())?;
    Ok(Arc::new(AppState::new(database, token, config)?))
}

async fn collecting(proxy: &Proxy, secret: &str) -> Result<Arc<AppState>, Failure> {
    let state = state(
        "cliproxy",
        &format!("http://127.0.0.1:{}", proxy.port),
        secret,
    )
    .await?;
    collector::spawn_all(Arc::clone(&state));
    Ok(state)
}

async fn serve_spend_log(log: SpendLog) -> Result<u16, Failure> {
    let acceptor = poem::listener::TcpListener::bind("127.0.0.1:0")
        .into_acceptor()
        .await?;
    let port = acceptor
        .local_addr()
        .first()
        .and_then(|address| address.as_socket_addr())
        .ok_or("the listener has no socket address")?
        .port();
    let app = Route::new()
        .at("/spend/logs/v2", poem::get(spend_logs))
        .data(Arc::new(log));
    tokio::spawn(Server::new_with_acceptor(acceptor).run(app));
    Ok(port)
}

async fn scalar(state: &AppState, sql: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&state.database.pool)
        .await
}

async fn copies_of(state: &AppState, request_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM usage_event WHERE upstream_id = ?")
        .bind(request_id)
        .fetch_one(&state.database.pool)
        .await
}

async fn eventually(what: &str, mut done: impl AsyncFnMut() -> bool) -> Result<(), Failure> {
    for _ in 0..500 {
        if done().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(format!("timed out waiting until {what}").into())
}

async fn events_are(state: &AppState, count: i64) -> bool {
    scalar(state, "SELECT COUNT(*) FROM usage_event")
        .await
        .is_ok_and(|stored| stored == count)
}

async fn has_failed(state: &AppState) -> bool {
    scalar(
        state,
        "SELECT COUNT(*) FROM collector_state WHERE consecutive_failures >= 1",
    )
    .await
    .is_ok_and(|failed| failed == 1)
}

#[tokio::test]
async fn buffered_and_pushed_records_are_stored_and_notices_are_not() -> Result<(), Failure> {
    let proxy = Proxy::start().await?;
    for id in ["buffered-1", "buffered-2", "buffered-3"] {
        proxy.enqueue(&record(id)?);
    }
    let state = collecting(&proxy, MANAGEMENT_KEY).await?;
    eventually("the buffer is drained", async || {
        events_are(&state, 3).await
    })
    .await?;
    eventually("the collector subscribes", async || {
        proxy.subscribers() == 1
    })
    .await?;

    proxy.enqueue(&record("pushed-1")?);
    proxy.enqueue(br#"{"refresh":true}"#);
    proxy.enqueue(&record("pushed-2")?);
    eventually("pushed records are stored", async || {
        events_are(&state, 5).await
    })
    .await?;

    assert_eq!(copies_of(&state, "pushed-1").await?, 1);
    assert_eq!(copies_of(&state, "pushed-2").await?, 1);
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM dead_letter").await?,
        0,
        "a queue notice is neither a record nor a refused one"
    );
    assert_eq!(
        scalar(&state, "SELECT records_received FROM collector_state").await?,
        5
    );
    assert_eq!(
        scalar(
            &state,
            "SELECT COUNT(*) FROM collector_state WHERE connected_since IS NOT NULL \
             AND last_record_at IS NOT NULL AND consecutive_failures = 0"
        )
        .await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn a_record_finished_during_the_drain_is_stored_once() -> Result<(), Failure> {
    let proxy = Proxy::start().await?;
    proxy.enqueue(&record("before")?);
    proxy.queue().during_drain = Some(record("during")?);
    let state = collecting(&proxy, MANAGEMENT_KEY).await?;

    eventually("both records are stored", async || {
        events_are(&state, 2).await
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(copies_of(&state, "before").await?, 1);
    assert_eq!(
        copies_of(&state, "during").await?,
        1,
        "a subscription open before the drain receives it; a drain before the subscription \
         would leave it buffered"
    );
    Ok(())
}

#[tokio::test]
async fn a_dropped_connection_is_reopened_and_what_buffered_meanwhile_is_drained()
-> Result<(), Failure> {
    let proxy = Proxy::start().await?;
    let state = collecting(&proxy, MANAGEMENT_KEY).await?;
    eventually("the collector subscribes", async || {
        proxy.subscribers() == 1
    })
    .await?;

    proxy.restart_with(&[record("while-away-1")?, record("while-away-2")?]);
    eventually("the buffer is drained after reconnecting", async || {
        events_are(&state, 2).await
    })
    .await?;
    eventually("the collector subscribes again", async || {
        proxy.subscribers() == 1
    })
    .await?;

    proxy.enqueue(&record("after")?);
    eventually("the new subscription delivers", async || {
        copies_of(&state, "after")
            .await
            .is_ok_and(|copies| copies == 1)
    })
    .await?;
    assert_eq!(copies_of(&state, "while-away-1").await?, 1);
    assert_eq!(copies_of(&state, "while-away-2").await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_refused_key_is_recorded_without_the_key() -> Result<(), Failure> {
    let proxy = Proxy::start().await?;
    let wrong = "wrong-management-key-93b1";
    let state = collecting(&proxy, wrong).await?;
    eventually("the failure is recorded", async || has_failed(&state).await).await?;

    let (error, connected) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT last_error, connected_since FROM collector_state",
    )
    .fetch_one(&state.database.pool)
    .await?;
    assert!(error.contains("AUTH was refused"), "{error}");
    assert!(!error.contains(wrong), "{error}");
    assert_eq!(connected, None);
    Ok(())
}

#[handler]
async fn spend_logs(
    request: &poem::Request,
    Query(query): Query<SpendQuery>,
    log: Data<&Arc<SpendLog>>,
) -> poem::Result<Json<Value>> {
    let expected = format!("Bearer {MASTER_KEY}");
    if request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return Err(poem::Error::from_status(StatusCode::UNAUTHORIZED));
    }
    let start = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", &query.start_date)
        .and_then(|start| start.to_zoned(jiff::tz::TimeZone::UTC))
        .map_err(|_| poem::Error::from_status(StatusCode::BAD_REQUEST))?
        .timestamp();
    let rows = log
        .rows
        .iter()
        .filter(|row| {
            row["startTime"]
                .as_str()
                .and_then(|text| text.parse::<Timestamp>().ok())
                .is_some_and(|at| at >= start)
        })
        .skip((query.page - 1) * query.page_size)
        .take(query.page_size)
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(json!({ "data": rows, "page": query.page })))
}

/// Rows a minute apart ending now, so a window that reaches 15 minutes back from the
/// newest overlaps the last fifteen of them. The last row failed and the one before it
/// read from and wrote to a cache.
fn spend_rows() -> Vec<Value> {
    let newest = Timestamp::now() - SignedDuration::from_mins(1);
    (0..ROWS)
        .map(|index| {
            let started = newest - SignedDuration::from_mins(ROWS - 1 - index);
            let mut row = json!({
                "request_id": format!("chatcmpl-{index}"),
                "call_type": "acompletion",
                "api_key": KEY_HASH,
                "spend": SPEND,
                "total_tokens": 150,
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "startTime": started.to_string(),
                "completionStartTime": (started + SignedDuration::from_millis(300)).to_string(),
                "model": "gpt-4o-2024-08-06",
                "model_id": "deployment-a",
                "model_group": "gpt-4o",
                "custom_llm_provider": "openai",
                "api_base": "https://api.openai.com",
                "status": "success",
                "request_duration_ms": 900,
                "metadata": { "user_api_key": KEY_HASH },
            });
            match index % 3 {
                0 => row["metadata"]["user_api_key_alias"] = json!("ci-bot"),
                1 => row["model_id"] = json!(""),
                _ => {
                    row["model_id"] = Value::Null;
                    row["api_base"] = Value::Null;
                }
            }
            if index == ROWS - 1 {
                row["status"] = json!("failure");
                row["metadata"]["error_information"] =
                    json!({ "error_code": "429", "error_message": "rate limited" });
            }
            if index == ROWS - 2 {
                row["metadata"]["additional_usage_values"] =
                    json!({ "cache_read_input_tokens": 40, "cache_creation_input_tokens": 10 });
            }
            row
        })
        .collect()
}

#[tokio::test]
async fn spend_logs_are_paged_attributed_and_read_once() -> Result<(), Failure> {
    let port = serve_spend_log(SpendLog { rows: spend_rows() }).await?;
    let state = state("litellm", &format!("http://127.0.0.1:{port}/"), MASTER_KEY).await?;
    let source = Source::by_key(&state.database, "gateway")
        .await?
        .ok_or("the configured source")?;
    let config = state
        .config
        .source("gateway")
        .ok_or("the configured source")?;
    let client = state.http.clone();

    let first = collector::litellm::poll(&state, &client, source.id, config).await?;
    assert_eq!((first.accepted, first.rejected), (ROWS, 0));

    let accounts = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT upstream_key, label FROM account ORDER BY upstream_key",
    )
    .fetch_all(&state.database.pool)
    .await?;
    let group = Some("gpt-4o".to_owned());
    assert_eq!(
        accounts,
        [
            ("deployment-a".to_owned(), group.clone()),
            ("https://api.openai.com".to_owned(), group.clone()),
            ("unattributed/openai".to_owned(), group),
        ]
    );
    let callers = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT caller, COUNT(*) FROM usage_event GROUP BY caller ORDER BY caller",
    )
    .fetch_all(&state.database.pool)
    .await?;
    assert_eq!(
        callers,
        [
            (Some("ci-bot".to_owned()), 168),
            (Some("…6b4b".to_owned()), 335)
        ]
    );
    let billed = sqlx::query_scalar::<_, f64>("SELECT SUM(billed_cost_usd) FROM usage_event")
        .fetch_one(&state.database.pool)
        .await?;
    assert!((billed - SPEND * 503.0).abs() < f64::EPSILON, "{billed}");
    let failed = sqlx::query_as::<_, (i64, bool, Option<i64>)>(
        "SELECT status_code, failed, ttft_ms FROM usage_event WHERE upstream_id = ?",
    )
    .bind(format!("chatcmpl-{}", ROWS - 1))
    .fetch_one(&state.database.pool)
    .await?;
    assert_eq!(failed, (429, true, Some(300)));
    let cached = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT input_tokens, cache_read_tokens, cache_write_tokens FROM usage_event \
         WHERE upstream_id = ?",
    )
    .bind(format!("chatcmpl-{}", ROWS - 2))
    .fetch_one(&state.database.pool)
    .await?;
    assert_eq!(cached, (100, 40, 10));

    let second = collector::litellm::poll(&state, &client, source.id, config).await?;
    assert_eq!(
        second.accepted, 0,
        "an overlapping window stores nothing again"
    );
    assert!(
        second.duplicates >= 15,
        "the window reached back: {second:?}"
    );
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM usage_event").await?,
        ROWS
    );
    assert_eq!(
        scalar(&state, "SELECT records_received FROM collector_state").await?,
        ROWS
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_master_key_is_recorded_without_the_key() -> Result<(), Failure> {
    let port = serve_spend_log(SpendLog::default()).await?;
    let wrong = "sk-wrong-master-5e2a";
    let state = state("litellm", &format!("http://127.0.0.1:{port}"), wrong).await?;
    collector::spawn_all(Arc::clone(&state));

    eventually("the failure is recorded", async || has_failed(&state).await).await?;
    let error = sqlx::query_scalar::<_, String>("SELECT last_error FROM collector_state")
        .fetch_one(&state.database.pool)
        .await?;
    assert!(error.contains("401"), "{error}");
    assert!(!error.contains(wrong), "{error}");
    Ok(())
}
