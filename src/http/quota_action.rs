//! Request validation and disconnect-independent execution of human quota actions.
use crate::app::AppState;
use crate::http::api::quota::QuotaApi;
use crate::http::auth::CredentialIdentity;
use crate::http::quota_action_store::{Claim, Invocation, json_response, reject};
use crate::prelude::*;
use futures_util::FutureExt;
use jiff::Timestamp;
use poem::http::StatusCode;
use poem::{Body, Endpoint, IntoResponse, Request, Response};
use serde_json::Value;
use std::sync::Arc;
/// App labels are bounded before entering logs and headers.
const CLIENT_LABEL_LIMIT: usize = 80;
/// End before common 60-second proxy timeouts. Provider sequences can take minutes.
const HTTP_WAIT: std::time::Duration = std::time::Duration::from_secs(50);
pub struct SharedAction<E> {
    endpoint: Arc<E>,
    kind: &'static str,
}

impl<E: Endpoint + 'static> Endpoint for SharedAction<E> {
    type Output = Response;

    async fn call(&self, request: Request) -> poem::Result<Response> {
        let worker = Self {
            endpoint: Arc::clone(&self.endpoint),
            kind: self.kind,
        };
        let (started, claimed) = tokio::sync::oneshot::channel();
        // The HTTP wait owns only a JoinHandle, never the operation future itself.
        let mut task = tokio::spawn(async move { worker.execute(request, started).await });
        let result = match claimed.await {
            Ok(pending) => match tokio::time::timeout(HTTP_WAIT, &mut task).await {
                Ok(result) => result,
                Err(_) => return Ok(pending),
            },
            Err(_) => task.await,
        };
        result.map_err(poem::error::InternalServerError)?
    }
}

impl<E: Endpoint> SharedAction<E> {
    async fn execute(
        &self,
        mut request: Request,
        started: tokio::sync::oneshot::Sender<Response>,
    ) -> poem::Result<Response> {
        let state = request
            .data::<Arc<AppState>>()
            .cloned()
            .ok_or_else(|| poem::Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
        let requested_at = Timestamp::now();
        let identity = request
            .data::<CredentialIdentity>()
            .copied()
            .ok_or_else(|| poem::Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
        let label = request
            .headers()
            .get("x-quota-client")
            .and_then(|v| v.to_str().ok());
        let valid_label = label.is_some_and(|v| {
            !v.trim().is_empty()
                && v.len() <= CLIENT_LABEL_LIMIT
                && v.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        });
        let caller = if valid_label {
            label.unwrap_or_default()
        } else {
            "invalid-or-missing"
        }
        .to_owned();
        let idempotency_key = request
            .headers()
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let invalid_key = request.headers().contains_key("idempotency-key")
            && !idempotency_key.as_deref().is_some_and(|v| {
                !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_graphic())
            });
        let body = request.take_body().into_bytes().await?;
        let input = serde_json::from_slice::<Value>(&body).ok();
        request.set_body(Body::from(body));
        let rejection = rejection(
            &request,
            self.kind,
            input.as_ref(),
            valid_label,
            invalid_key,
        );
        if let Some((status, message)) = rejection {
            return reject(&state, self.kind, &identity, &caller, status, message)
                .await
                .map_err(poem::error::InternalServerError);
        }
        let input = input.unwrap_or_default();
        let account_id = (self.kind != "soft_refresh")
            .then(|| input.get("account_id").and_then(Value::as_str))
            .flatten();
        let source_id = (self.kind == "soft_refresh")
            .then(|| input.get("source_id").and_then(Value::as_str))
            .flatten();
        if identity.transport == "internal" && self.kind == "hard_refresh" {
            let Some(account_id) = account_id.and_then(|id| id.parse::<Id<Account>>().ok()) else {
                return Ok(Response::builder().status(StatusCode::NO_CONTENT).finish());
            };
            if !crate::quota::recovery::Recovery::claim_due(
                &state.database,
                account_id,
                Timestamp::now(),
            )
            .await
            .map_err(poem::error::InternalServerError)?
            {
                return Ok(Response::builder().status(StatusCode::NO_CONTENT).finish());
            }
        }
        let api = QuotaApi::new(Arc::clone(&state));
        let result = self
            .run(
                &state,
                &api,
                request,
                started,
                Invocation {
                    account_id,
                    source_id,
                    caller: &caller,
                    requested_at,
                    identity: &identity,
                    idempotency_key: idempotency_key.as_deref(),
                },
            )
            .await;
        result.map_err(poem::error::InternalServerError)
    }
}

impl<E: Endpoint> SharedAction<E> {
    async fn run(
        &self,
        state: &AppState,
        api: &QuotaApi,
        request: Request,
        started: tokio::sync::oneshot::Sender<Response>,
        invocation: Invocation<'_>,
    ) -> Result<Response, DatabaseError> {
        let mut action = match invocation.claim(state, api, self.kind).await? {
            Claim::Reused(mut action) => {
                let _ = started.send(action.accepted(true));
                if let Ok(result) =
                    tokio::time::timeout(HTTP_WAIT, action.wait_for_completion(state)).await
                {
                    result?;
                }
                return action.replay(api).await;
            }
            Claim::Fresh(action) => action,
            Claim::Conflict => {
                return reject(
                    state,
                    self.kind,
                    invocation.identity,
                    invocation.caller,
                    StatusCode::CONFLICT,
                    "Idempotency-Key was already used for another operation or scope",
                )
                .await;
            }
        };
        let _ = started.send(action.accepted(false));
        // Claim first so callers queued behind another operation already have an ID.
        // SQLite serializes claims; only fresh work takes the provider-family lock.
        let family = match self.kind {
            "soft_refresh" => 0,
            "cooldown_reset" => 1,
            _ => 2,
        };
        let _guard = state.quota_action_locks[family].lock().await;
        let execution = std::panic::AssertUnwindSafe(async {
            let response = match self.endpoint.call(request).await {
                Ok(response) => response.into_response(),
                Err(error) => error.into_response(),
            };
            json_response(response).await
        });
        let (status, body) = match execution.catch_unwind().await {
            Ok(result) => result,
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"message":"Action interrupted"}"#.to_owned(),
            ),
        };
        action
            .complete(state, api, &invocation, status, &body)
            .await;
        Ok(action.respond(status, body, false))
    }
}

pub fn share_soft_refresh<E: Endpoint + 'static>(endpoint: E) -> SharedAction<E> {
    SharedAction {
        endpoint: Arc::new(endpoint),
        kind: "soft_refresh",
    }
}
pub fn share_hard_refresh<E: Endpoint + 'static>(endpoint: E) -> SharedAction<E> {
    SharedAction {
        endpoint: Arc::new(endpoint),
        kind: "hard_refresh",
    }
}
pub fn share_quota_reset<E: Endpoint + 'static>(endpoint: E) -> SharedAction<E> {
    SharedAction {
        endpoint: Arc::new(endpoint),
        kind: "quota_reset",
    }
}
pub fn share_cooldown_reset<E: Endpoint + 'static>(endpoint: E) -> SharedAction<E> {
    SharedAction {
        endpoint: Arc::new(endpoint),
        kind: "cooldown_reset",
    }
}

fn rejection(
    request: &Request,
    kind: &str,
    input: Option<&Value>,
    valid_label: bool,
    invalid_key: bool,
) -> Option<(StatusCode, &'static str)> {
    let content_type = request
        .header("content-type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    let content_is_json = content_type == "application/json"
        || content_type.starts_with("application/") && content_type.ends_with("+json");
    if !valid_label {
        Some((
            StatusCode::BAD_REQUEST,
            "X-Quota-Client is required and must be 1 to 80 printable ASCII characters",
        ))
    } else if invalid_key {
        Some((
            StatusCode::BAD_REQUEST,
            "Idempotency-Key must be 1 to 128 graphic ASCII characters",
        ))
    } else if !content_is_json {
        Some((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ))
    } else if !input.is_some_and(|v| {
        v.is_object()
            && ["account_id", "source_id"]
                .iter()
                .all(|key| v.get(*key).is_none_or(|v| v.is_null() || v.is_string()))
    }) {
        Some((
            StatusCode::BAD_REQUEST,
            "a JSON object with string scope ids is required",
        ))
    } else if kind == "cooldown_reset"
        && input.is_none_or(|v| v["account_id"].as_str().is_none_or(str::is_empty))
    {
        Some((
            StatusCode::BAD_REQUEST,
            "a cooldown reset requires account_id",
        ))
    } else if kind == "quota_reset"
        && !input
            .as_ref()
            .is_some_and(|v| v["confirmed"] == true && v["account_id"].is_string())
    {
        Some((
            StatusCode::BAD_REQUEST,
            "a manual reset requires account_id and confirmed true",
        ))
    } else {
        None
    }
}

/// Internal recovery uses the exact human-action claim and completion path, without
/// fabricating an admin credential or exposing an unauthenticated HTTP route.
pub async fn automatic_recovery(state: Arc<AppState>, account_id: Id<Account>) {
    let endpoint = poem::endpoint::make(move |request: Request| async move {
        let state = request
            .data::<Arc<AppState>>()
            .ok_or_else(|| poem::Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?
            .clone();
        Ok::<_, poem::Error>(QuotaApi::new(state).recovery_refresh(account_id).await)
    });
    let mut request = Request::builder()
        .method(poem::http::Method::POST)
        .header("Content-Type", "application/json")
        .header("X-Quota-Client", "automatic recovery")
        .body(serde_json::json!({"account_id": account_id.encode()}).to_string());
    request.extensions_mut().insert(state);
    request.extensions_mut().insert(CredentialIdentity {
        credential_id: "internal:automatic-recovery",
        transport: "internal",
    });
    if let Err(error) = share_hard_refresh(endpoint).call(request).await {
        tracing::error!(%error, %account_id, "automatic recovery failed");
    }
}
