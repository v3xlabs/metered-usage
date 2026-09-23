//! Reads CLIProxyAPI's usage queue over the RESP dialect it serves on its proxy port.
//!
//! The queue is destructive and has no cursor: a pop removes a record, and while any
//! subscriber exists a new record goes only to subscribers and is never buffered. So the
//! subscription opens first, and only once it is confirmed does a second connection pop
//! what was buffered before it. A record that arrives during that drain reaches the
//! subscription and waits in its channel, and nothing falls between the two.

use std::convert::Infallible;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::app::AppState;
use crate::collector::resp::{Connection, Reply, RespError};
use crate::collector::{Backoff, CollectorError, Progress};
use crate::config::{Secret, SourceConfig};
use crate::id::Id;
use crate::source::Source;
use crate::usage::cliproxy;
use crate::usage::ingest::{Incoming, ingest};

const CHANNEL: &[u8] = b"usage";
const POP_COUNT: &[u8] = b"512";
const BATCH_WINDOW: Duration = Duration::from_millis(250);
const BATCH_RECORDS: usize = 256;
/// Pushed records wait here while the drain runs. When it is full the reader stops
/// reading, the server's own buffer fills, and the server drops the subscription, which
/// ends in a reconnect and a drain rather than a lost record.
const PUSH_BACKLOG: usize = 4096;

/// A payload received but not yet committed, kept across a reconnect so a failed commit
/// loses nothing the destructive pop already removed upstream.
type Unsaved = Vec<Result<Value, String>>;

/// Stops the subscription reader when the session that owns it ends.
struct Reader(JoinHandle<()>);

impl Drop for Reader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn run(state: &AppState, source_id: Id<Source>, config: &SourceConfig) {
    let progress = Progress { source_id };
    let mut backoff = Backoff::default();
    let mut unsaved = Unsaved::new();
    loop {
        let Err(error) = session(state, config, progress, &mut backoff, &mut unsaved).await;
        progress
            .retry_after(&state.database, &config.key, &error, &mut backoff)
            .await;
    }
}

async fn session(
    state: &AppState,
    config: &SourceConfig,
    progress: Progress,
    backoff: &mut Backoff,
    unsaved: &mut Unsaved,
) -> Result<Infallible, CollectorError> {
    let (host, port) = address(&config.base_url)?;
    let mut subscriber = Connection::connect(&host, port).await?;
    authenticate(&mut subscriber, &config.secret).await?;
    match subscriber.command(&[b"SUBSCRIBE", CHANNEL]).await? {
        Reply::Array(Some(items)) if matches!(items.first(), Some(Reply::Bulk(Some(kind))) if kind == b"subscribe") =>
            {}
        Reply::Error(message) => {
            return Err(CollectorError::Refused {
                command: "SUBSCRIBE",
                message,
            });
        }
        other => {
            return Err(CollectorError::Unexpected {
                command: "SUBSCRIBE",
                reply: other.kind(),
            });
        }
    }
    progress.connected(&state.database).await?;
    backoff.reset();
    tracing::info!(source = config.key, "subscribed to the usage queue");

    let (sender, mut pushed) = mpsc::channel(PUSH_BACKLOG);
    let _reader = Reader(tokio::spawn(read_pushed(subscriber, sender)));

    store(state, progress, unsaved).await?;
    let mut popper = Connection::connect(&host, port).await?;
    authenticate(&mut popper, &config.secret).await?;
    loop {
        let items = match popper.command(&[b"LPOP", CHANNEL, POP_COUNT]).await? {
            Reply::Array(Some(items)) => items,
            Reply::Array(None) | Reply::Bulk(None) => Vec::new(),
            Reply::Error(message) => {
                return Err(CollectorError::Refused {
                    command: "LPOP",
                    message,
                });
            }
            other => {
                return Err(CollectorError::Unexpected {
                    command: "LPOP",
                    reply: other.kind(),
                });
            }
        };
        if items.is_empty() {
            break;
        }
        for item in items {
            let Reply::Bulk(Some(payload)) = item else {
                return Err(CollectorError::Unexpected {
                    command: "LPOP",
                    reply: item.kind(),
                });
            };
            keep(unsaved, &payload);
        }
        store(state, progress, unsaved).await?;
    }
    drop(popper);

    loop {
        let first = pushed.recv().await.ok_or(RespError::Closed)??;
        keep(unsaved, &first);
        let deadline = Instant::now() + BATCH_WINDOW;
        while unsaved.len() < BATCH_RECORDS {
            match tokio::time::timeout_at(deadline, pushed.recv()).await {
                Ok(Some(payload)) => keep(unsaved, &payload?),
                Ok(None) => return Err(RespError::Closed.into()),
                Err(_) => break,
            }
        }
        store(state, progress, unsaved).await?;
    }
}

/// Forwards each pushed payload until the connection fails, then forwards the failure.
async fn read_pushed(
    mut subscriber: Connection,
    sender: mpsc::Sender<Result<Vec<u8>, CollectorError>>,
) {
    loop {
        let payload = match subscriber.read().await {
            Ok(reply) => message(reply),
            Err(error) => Err(error.into()),
        };
        let failed = payload.is_err();
        if sender.send(payload).await.is_err() || failed {
            return;
        }
    }
}

fn message(reply: Reply) -> Result<Vec<u8>, CollectorError> {
    let kind = reply.kind();
    if let Reply::Array(Some(items)) = reply
        && let Ok([Reply::Bulk(Some(frame)), _, Reply::Bulk(Some(payload))]) =
            <[Reply; 3]>::try_from(items)
        && frame == b"message"
    {
        return Ok(payload);
    }
    Err(CollectorError::Unexpected {
        command: "SUBSCRIBE",
        reply: kind,
    })
}

async fn authenticate(connection: &mut Connection, secret: &Secret) -> Result<(), CollectorError> {
    match connection
        .command(&[b"AUTH", secret.expose().as_bytes()])
        .await?
    {
        Reply::Simple(_) => Ok(()),
        Reply::Error(message) => Err(CollectorError::Refused {
            command: "AUTH",
            message,
        }),
        other => Err(CollectorError::Unexpected {
            command: "AUTH",
            reply: other.kind(),
        }),
    }
}

/// Holds a payload for the next commit unless it is one of the queue's own notices.
fn keep(unsaved: &mut Unsaved, payload: &[u8]) {
    match serde_json::from_slice::<Value>(payload) {
        Ok(value) if is_control(&value) => {}
        Ok(value) => unsaved.push(Ok(value)),
        Err(error) => unsaved.push(Err(format!("the payload is not JSON: {error}"))),
    }
}

/// `{"support_refresh":true}` opens every subscription and `{"refresh":true}` follows a
/// credential reload; neither is a request.
fn is_control(value: &Value) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.len() == 1
            && (fields.contains_key("support_refresh") || fields.contains_key("refresh"))
    })
}

async fn store(
    state: &AppState,
    progress: Progress,
    unsaved: &mut Unsaved,
) -> Result<(), CollectorError> {
    if unsaved.is_empty() {
        return Ok(());
    }
    let records = unsaved
        .iter()
        .map(|payload| match payload {
            Ok(value) => cliproxy::parse(value.clone()),
            // The text that failed to parse may hold a raw API key, so none of it is kept.
            Err(error) => Incoming::Rejected {
                error: error.clone(),
                payload: Value::Null,
            },
        })
        .collect();
    let report = ingest(state, progress.source_id, records).await?;
    unsaved.clear();
    progress.received(&state.database, &report).await?;
    Ok(())
}

/// A bracketed IPv6 host is how a URL writes it, not how a socket takes it.
fn address(base_url: &str) -> Result<(String, u16), CollectorError> {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| {
            let host = url
                .host_str()?
                .trim_start_matches('[')
                .trim_end_matches(']');
            Some((host.to_owned(), url.port_or_known_default()?))
        })
        .ok_or_else(|| CollectorError::Address(base_url.to_owned()))
}
