use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, BoxStream, StreamExt};
use jiff::Timestamp;
use poem::web::sse::Event;
use poem_openapi::param::{Header, Query};
use poem_openapi::payload::{EventStream, Json};
use poem_openapi::types::ToJSON;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi, Union};
use tokio::sync::broadcast;

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::account::AccountSummaryOutput;
use crate::prelude::*;
use crate::usage::{PricedEvent, TokenQuality, UsageUpdate};

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 500;
const KEEP_ALIVE: Duration = Duration::from_secs(15);
const USAGE_EVENT: &str = "usage";
const PRICED_EVENT: &str = "priced";

pub struct UsageApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl UsageApi {
    #[oai(
        path = "/usage/events",
        method = "get",
        operation_id = "list_usage_events"
    )]
    async fn list_usage_events(
        &self,
        from: Query<Option<String>>,
        to: Query<Option<String>>,
        source_id: Query<Option<String>>,
        model: Query<Option<String>>,
        account_id: Query<Option<String>>,
        failed: Query<Option<bool>>,
        limit: Query<Option<i64>>,
        before: Query<Option<String>>,
        after: Query<Option<String>>,
    ) -> EventPageResponse {
        let query = Filter {
            from: from.0,
            to: to.0,
            source_id: source_id.0,
            model: model.0,
            account_id: account_id.0,
            failed: failed.0,
        };
        let filter = match UsageFilter::try_from(query) {
            Ok(filter) => filter,
            Err(error) => return EventPageResponse::Invalid(Json(error)),
        };
        let limit = limit.0.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let cursor = match (
            event_id(before.0.as_deref(), "before"),
            event_id(after.0.as_deref(), "after"),
        ) {
            (Err(error), _) | (_, Err(error)) => return EventPageResponse::Invalid(Json(error)),
            (Ok(Some(_)), Ok(Some(_))) => {
                return EventPageResponse::Invalid(Json(Error {
                    message: "before and after cannot be combined".to_owned(),
                }));
            }
            (Ok(before), Ok(after)) => before.map(Cursor::Before).or(after.map(Cursor::After)),
        };

        match UsageEvent::page(&self.state.database, &filter, limit, cursor).await {
            Ok(events) => {
                let next_before = (!matches!(cursor, Some(Cursor::After(_)))
                    && i64::try_from(events.len()) == Ok(limit))
                .then(|| events.last().map(|event| event.id.encode()))
                .flatten();
                EventPageResponse::Found(Json(EventPage {
                    events: events.into_iter().map(UsageEventOutput::from).collect(),
                    next_before,
                }))
            }
            Err(error) => EventPageResponse::Failed(Json(failure("list_usage_events", &error))),
        }
    }

    /// Server-Sent Events of two types. `usage` carries each event as it is committed,
    /// with the event id as its SSE id. `priced` carries a `PricedEvent` each time a price
    /// fill, periodic or a reprice, commits a cost for an event; it has no SSE id, so
    /// `Last-Event-ID` keeps naming the last `usage` message. Given `Last-Event-ID`, the
    /// stream first replays up to 500 stored events newer than that id, oldest first, as
    /// they now read. A client that falls too far behind is disconnected and is expected
    /// to reconnect with `Last-Event-ID`.
    #[oai(
        path = "/usage/stream",
        method = "get",
        operation_id = "stream_usage_events"
    )]
    async fn stream_usage_events(
        &self,
        #[oai(name = "Last-Event-ID")] last_event_id: Header<Option<String>>,
    ) -> StreamResponse {
        let last_event_id = match event_id(last_event_id.0.as_deref(), "Last-Event-ID") {
            Ok(last_event_id) => last_event_id,
            Err(error) => return StreamResponse::Invalid(Json(error)),
        };
        // Subscribing before the replay reads means nothing committed between the two is
        // missed; what both saw is dropped from the live half below.
        let receiver = self.state.usage.subscribe();
        let replayed = match last_event_id {
            None => Vec::new(),
            Some(after) => match UsageEvent::page(
                &self.state.database,
                &UsageFilter::default(),
                MAX_LIMIT,
                Some(Cursor::After(after)),
            )
            .await
            {
                Ok(events) => events,
                Err(error) => {
                    return StreamResponse::Failed(Json(failure("stream_usage_events", &error)));
                }
            },
        };
        let replayed_through = replayed.last().map(|event| event.id);
        let messages = stream::iter(replayed)
            .map(|event| UsageUpdate::Recorded(Box::new(event)))
            .chain(live(receiver, replayed_through))
            .map(StreamMessage::from)
            .boxed();

        StreamResponse::Live(EventStream::new(messages).keep_alive(KEEP_ALIVE).to_event(
            |message| {
                match message {
                    StreamMessage::Usage(event) => Event::message(event.to_json_string())
                        .id(event.event_id.clone())
                        .event_type(USAGE_EVENT),
                    StreamMessage::Priced(priced) => {
                        Event::message(priced.to_json_string()).event_type(PRICED_EVENT)
                    }
                }
            },
        ))
    }
}

/// The query as it arrived, before anything was read out of it.
struct Filter {
    from: Option<String>,
    to: Option<String>,
    source_id: Option<String>,
    model: Option<String>,
    account_id: Option<String>,
    failed: Option<bool>,
}

impl TryFrom<Filter> for UsageFilter {
    type Error = Error;

    fn try_from(query: Filter) -> Result<Self, Self::Error> {
        Ok(Self {
            from: instant(query.from.as_deref(), "from")?,
            to: instant(query.to.as_deref(), "to")?,
            source_id: match query.source_id.as_deref() {
                None => None,
                Some(source_id) => Some(source_id.parse().map_err(|_| Error {
                    message: "source_id must be a source id".to_owned(),
                })?),
            },
            model: query.model,
            account_id: match query.account_id.as_deref() {
                None => None,
                Some(account_id) => Some(account_id.parse().map_err(|_| Error {
                    message: "account_id must be an account id".to_owned(),
                })?),
            },
            failed: query.failed,
        })
    }
}

#[derive(Debug, Object)]
#[oai(rename = "UsageEvent", skip_serializing_if_is_none)]
struct UsageEventOutput {
    event_id: String,
    source_id: String,
    upstream_id: Option<String>,
    occurred_at: String,
    provider: String,
    model: String,
    model_alias: Option<String>,
    endpoint: String,
    account: AccountSummaryOutput,
    /// The client credential the request arrived with, masked.
    caller: Option<String>,
    harness: Option<String>,
    user_agent: Option<String>,
    session_id: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    unclassified_tokens: i64,
    total_tokens: i64,
    token_quality: Option<TokenQualityOutput>,
    latency_ms: Option<i64>,
    ttft_ms: Option<i64>,
    streamed: bool,
    status_code: i64,
    failed: bool,
    error_message: Option<String>,
    service_tier: Option<String>,
    reasoning_effort: Option<String>,
    list_cost_usd: Option<f64>,
    billed_cost_usd: Option<f64>,
}

impl From<UsageEvent> for UsageEventOutput {
    fn from(event: UsageEvent) -> Self {
        Self {
            event_id: event.id.encode(),
            source_id: event.source_id.encode(),
            // The column is never NULL, and an upstream that named no request wrote the
            // empty string into it, which is an absent id and not an id of no characters.
            upstream_id: Some(event.upstream_id).filter(|id| !id.is_empty()),
            occurred_at: event.occurred_at.to_string(),
            provider: event.provider,
            model: event.model,
            model_alias: event.model_alias,
            endpoint: event.endpoint,
            account: event.account.into(),
            caller: event.caller,
            harness: event.harness,
            user_agent: event.user_agent,
            session_id: event.session_id,
            input_tokens: event.input_tokens,
            output_tokens: event.output_tokens,
            reasoning_tokens: event.reasoning_tokens,
            cache_read_tokens: event.cache_read_tokens,
            cache_write_tokens: event.cache_write_tokens,
            unclassified_tokens: event.unclassified_tokens,
            total_tokens: event.total_tokens,
            token_quality: event.token_quality.map(TokenQualityOutput::from),
            latency_ms: event.latency_ms,
            ttft_ms: event.ttft_ms,
            streamed: event.streamed,
            status_code: event.status_code,
            failed: event.failed,
            error_message: event.error_message,
            service_tier: event.service_tier,
            reasoning_effort: event.reasoning_effort,
            list_cost_usd: event.list_cost_usd,
            billed_cost_usd: event.billed_cost_usd,
        }
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "TokenQuality", rename_all = "snake_case")]
enum TokenQualityOutput {
    Complete,
    Unclassified,
    Inconsistent,
}

impl From<TokenQuality> for TokenQualityOutput {
    fn from(quality: TokenQuality) -> Self {
        match quality {
            TokenQuality::Complete => Self::Complete,
            TokenQuality::Unclassified => Self::Unclassified,
            TokenQuality::Inconsistent => Self::Inconsistent,
        }
    }
}

#[derive(Debug, Object)]
#[oai(skip_serializing_if_is_none)]
struct EventPage {
    events: Vec<UsageEventOutput>,
    next_before: Option<String>,
}

/// One Server-Sent Event of the usage stream; its SSE type says which.
#[derive(Debug, Union)]
#[oai(rename = "UsageStreamMessage", one_of)]
enum StreamMessage {
    Usage(Box<UsageEventOutput>),
    Priced(PricedEventOutput),
}

impl From<UsageUpdate> for StreamMessage {
    fn from(update: UsageUpdate) -> Self {
        match update {
            UsageUpdate::Recorded(event) => Self::Usage(Box::new((*event).into())),
            UsageUpdate::Priced(priced) => Self::Priced(priced.into()),
        }
    }
}

/// The cost a price fill committed for an event already sent.
#[derive(Debug, Object)]
#[oai(rename = "PricedEvent")]
struct PricedEventOutput {
    event_id: String,
    price_id: String,
    list_cost_usd: f64,
    billed_cost_usd: f64,
}

impl From<PricedEvent> for PricedEventOutput {
    fn from(priced: PricedEvent) -> Self {
        Self {
            event_id: priced.event_id.encode(),
            price_id: priced.price_id.encode(),
            list_cost_usd: priced.list_cost_usd,
            billed_cost_usd: priced.billed_cost_usd,
        }
    }
}

#[derive(ApiResponse)]
enum EventPageResponse {
    #[oai(status = 200)]
    Found(Json<EventPage>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum StreamResponse {
    #[oai(status = 200)]
    Live(EventStream<BoxStream<'static, StreamMessage>>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn event_id(value: Option<&str>, name: &'static str) -> Result<Option<Id<UsageEvent>>, Error> {
    value
        .map(str::parse::<Id<UsageEvent>>)
        .transpose()
        .map_err(|_| Error {
            message: format!("{name} must be an event id"),
        })
}

/// Committed updates as they are published, without any event the replay already sent.
/// A receiver that lagged has lost updates it cannot name, so the stream ends there
/// rather than skip them.
fn live(
    receiver: broadcast::Receiver<UsageUpdate>,
    replayed_through: Option<Id<UsageEvent>>,
) -> BoxStream<'static, UsageUpdate> {
    stream::unfold(receiver, move |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(UsageUpdate::Recorded(event))
                    if replayed_through.is_some_and(|through| event.id <= through) => {}
                Ok(update) => return Some((update, receiver)),
                Err(_) => return None,
            }
        }
    })
    .boxed()
}

fn instant(value: Option<&str>, name: &'static str) -> Result<Option<Timestamp>, Error> {
    value
        .map(str::parse::<Timestamp>)
        .transpose()
        .map_err(|_| Error {
            message: format!("{name} must be an RFC 3339 timestamp"),
        })
}

fn failure(operation: &'static str, error: &DatabaseError) -> Error {
    tracing::error!(%operation, %error, "usage query failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
