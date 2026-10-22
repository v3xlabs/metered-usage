use std::sync::Arc;
use std::time::Duration;

use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};
use tokio::time::Instant;

use crate::app::AppState;
use crate::change::{Change, ChangeKind, Page, Subject, UnknownChangeKind};
use crate::http::api::Error;
use crate::prelude::*;

/// How long a read without `timeout` waits for a change.
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
/// The longest a read waits. A reverse proxy closes an upstream that sends nothing for a
/// minute by default (nginx's `proxy_read_timeout`), so a wait has to end before that.
const MAX_TIMEOUT_SECONDS: u64 = 50;

pub struct ChangeApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl ChangeApi {
    /// Changes after `cursor` to quota, resets, prices, accounts, plans and usage, and
    /// request arrivals, oldest first, as a long poll. With no changes, waits up to
    /// `timeout` seconds, default 30 and at most 50, and answers when a change commits.
    ///
    /// `quota.updated` names an account and window whose reported values changed;
    /// `reset.updated` an account whose manual resets changed; `account.changed` an
    /// account whose names, CLIProxy state or last hard refresh changed; `price.changed`
    /// a model whose latest price changed; `plan.changed` a plan added, edited or removed.
    /// `usage.updated` has no subject fields. It is coalesced to one event per accepted
    /// ingest batch, nonempty price fill, reprice transaction or account merge moving usage.
    /// Read the usage summary and leverage again. Duplicates and rejected records alone
    /// do not emit it. Predictions can move with usage or time without a `quota.updated`.
    /// `request.arrived` records a newly accepted request that happened in the last 24
    /// hours, the longest `/analytics/health` window, with only `account_id` and `failed`
    /// as subject fields. Its cursor identifies the arrival. It commits with ingestion,
    /// after the batch's `account.changed` and in accepted-record order. Older backfilled
    /// requests, duplicates, rejected records, repricing and account merges record none.
    /// An `account_id` missing from `/accounts` is new: read `/accounts` again. One still
    /// missing was merged into another account since the arrival. Read `/analytics/health`
    /// for the request buckets.
    ///
    /// `kinds` is a comma-separated list of the kinds to answer; without it every kind but
    /// `request.arrived` is answered. Read on from the `cursor` of the answer, which passes
    /// over the changes `kinds` left out.
    ///
    /// Without `cursor`, answers at once with no changes and the latest cursor: take it,
    /// read the lists, then follow from it. Changes are kept for a week. A cursor older
    /// than that, or ahead of the latest one, answers 410 with `oldest_cursor` and
    /// `latest_cursor`: read the lists again and follow from `latest_cursor`.
    /// Answers JSON pages of at most 500 events, not SSE. Arrival cursors increase in
    /// commit order, not request occurrence time, and can have gaps from other kinds.
    #[oai(path = "/events", method = "get", operation_id = "list_changes")]
    async fn list_changes(
        &self,
        cursor: Query<Option<i64>>,
        kinds: Query<Option<String>>,
        timeout: Query<Option<u64>>,
    ) -> ChangesResponse {
        let kinds = match change_kinds(kinds.0.as_deref()) {
            Ok(kinds) => kinds,
            Err(error) => {
                return ChangesResponse::Invalid(Json(Error {
                    message: error.to_string(),
                }));
            }
        };
        let database = &self.state.database;
        let Some(cursor) = cursor.0 else {
            return match Change::latest(database).await {
                Ok(latest) => ChangesResponse::Found(Json(ChangesOutput {
                    events: Vec::new(),
                    cursor: latest,
                })),
                Err(error) => ChangesResponse::Failed(Json(failure(&error))),
            };
        };
        if cursor < 0 {
            return ChangesResponse::Invalid(Json(Error {
                message: "cursor must not be negative".to_owned(),
            }));
        }
        let timeout = timeout
            .0
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .min(MAX_TIMEOUT_SECONDS);
        let until = Instant::now() + Duration::from_secs(timeout);
        match Change::wait(database, cursor, &kinds, until).await {
            Ok(Page::Changes { changes, cursor }) => ChangesResponse::Found(Json(ChangesOutput {
                events: changes.into_iter().map(ChangeOutput::from).collect(),
                cursor,
            })),
            Ok(Page::Expired { oldest, latest }) => ChangesResponse::Expired(Json(ExpiredOutput {
                message: "the cursor is older than the oldest change kept, or ahead of the \
                          latest; read the lists again and follow from latest_cursor"
                    .to_owned(),
                oldest_cursor: oldest,
                latest_cursor: latest,
            })),
            Err(error) => ChangesResponse::Failed(Json(failure(&error))),
        }
    }
}

/// One change. Only the fields its kind names are present.
#[derive(Debug, Object)]
#[oai(rename = "ChangeEvent", skip_serializing_if_is_none)]
struct ChangeOutput {
    cursor: i64,
    kind: ChangeKindOutput,
    recorded_at: String,
    /// The account of a `quota.updated`, `reset.updated`, `account.changed` or `request.arrived`.
    account_id: Option<String>,
    /// The window of a `quota.updated`.
    window_key: Option<String>,
    /// The model of a `price.changed`.
    model: Option<String>,
    /// The plan of a `plan.changed`, including after deletion.
    plan_id: Option<String>,
    /// Present only for `request.arrived`, including false for a successful request.
    failed: Option<bool>,
}

impl From<Change> for ChangeOutput {
    fn from(change: Change) -> Self {
        let kind = change.subject.kind().into();
        let (account_id, window_key, model, plan_id, failed) = match change.subject {
            Subject::Account(account_id) | Subject::Resets(account_id) => {
                (Some(account_id.encode()), None, None, None, None)
            }
            Subject::Window {
                account_id,
                window_key,
            } => (
                Some(account_id.encode()),
                Some(window_key),
                None,
                None,
                None,
            ),
            Subject::Price { model } => (None, None, Some(model), None, None),
            Subject::Plan(plan_id) => (None, None, None, Some(plan_id.encode()), None),
            Subject::Usage => (None, None, None, None, None),
            Subject::Arrival { account_id, failed } => {
                (Some(account_id.encode()), None, None, None, Some(failed))
            }
        };
        Self {
            cursor: change.cursor,
            kind,
            recorded_at: change.recorded_at.to_string(),
            account_id,
            window_key,
            model,
            plan_id,
            failed,
        }
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "ChangeKind")]
enum ChangeKindOutput {
    #[oai(rename = "quota.updated")]
    QuotaUpdated,
    #[oai(rename = "reset.updated")]
    ResetUpdated,
    #[oai(rename = "price.changed")]
    PriceChanged,
    #[oai(rename = "account.changed")]
    AccountChanged,
    #[oai(rename = "plan.changed")]
    PlanChanged,
    #[oai(rename = "usage.updated")]
    UsageUpdated,
    #[oai(rename = "request.arrived")]
    RequestArrived,
}

impl From<ChangeKind> for ChangeKindOutput {
    fn from(kind: ChangeKind) -> Self {
        match kind {
            ChangeKind::QuotaUpdated => Self::QuotaUpdated,
            ChangeKind::ResetUpdated => Self::ResetUpdated,
            ChangeKind::PriceChanged => Self::PriceChanged,
            ChangeKind::AccountChanged => Self::AccountChanged,
            ChangeKind::PlanChanged => Self::PlanChanged,
            ChangeKind::UsageUpdated => Self::UsageUpdated,
            ChangeKind::RequestArrived => Self::RequestArrived,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "ChangeEvents")]
struct ChangesOutput {
    events: Vec<ChangeOutput>,
    /// The cursor to send on the next read.
    cursor: i64,
}

#[derive(Debug, Object)]
#[oai(rename = "ChangesExpired")]
struct ExpiredOutput {
    message: String,
    /// The lowest cursor still answered.
    oldest_cursor: i64,
    /// The cursor to follow from after reading the lists again.
    latest_cursor: i64,
}

#[derive(ApiResponse)]
enum ChangesResponse {
    #[oai(status = 200)]
    Found(Json<ChangesOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 410)]
    Expired(Json<ExpiredOutput>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

/// The kinds a comma-separated list names. An empty list names none, which reads as
/// [`ChangeKind::DEFAULT`].
fn change_kinds(list: Option<&str>) -> Result<Vec<ChangeKind>, UnknownChangeKind> {
    let kinds = list
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if kinds.is_empty() {
        return Ok(ChangeKind::DEFAULT.to_vec());
    }

    Ok(kinds)
}

fn failure(error: &DatabaseError) -> Error {
    tracing::error!(operation = "list_changes", %error, "change operation failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
