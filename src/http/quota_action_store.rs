//! Durable claims, replay and completion for human quota actions.
use crate::app::AppState;
use crate::change::{Change, Subject};
use crate::http::api::quota::QuotaApi;
use crate::http::auth::CredentialIdentity;
use crate::prelude::*;
use crate::quota::reuse::REUSE_FOR;
use jiff::Timestamp;
use poem::Response;
use poem::http::{HeaderValue, StatusCode};
use serde_json::{Value, json};
use sqlx::FromRow;

pub enum Claim {
    Fresh(SavedAction),
    Reused(SavedAction),
    Conflict,
}

#[derive(Clone, Copy)]
pub struct Invocation<'a> {
    pub account_id: Option<&'a str>,
    pub source_id: Option<&'a str>,
    pub caller: &'a str,
    pub requested_at: Timestamp,
    pub identity: &'a CredentialIdentity,
    pub idempotency_key: Option<&'a str>,
}

impl Invocation<'_> {
    async fn record_key(
        &self,
        write: &mut crate::database::Write<'_>,
        action_id: i64,
        kind: &str,
    ) -> Result<bool, DatabaseError> {
        if let Some(key) = self.idempotency_key {
            let inserted = sqlx::query("INSERT INTO quota_action_key (credential_id, key, kind, account_id, source_id, action_id) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING")
                .bind(self.identity.credential_id).bind(key).bind(kind).bind(self.account_id).bind(self.source_id).bind(action_id).execute(&mut **write).await?;
            if inserted.rows_affected() == 0 {
                let bound: i64 = sqlx::query_scalar(
                    "SELECT action_id FROM quota_action_key WHERE credential_id = ? AND key = ?",
                )
                .bind(self.identity.credential_id)
                .bind(key)
                .fetch_one(&mut **write)
                .await?;
                return Ok(bound == action_id);
            }
        }
        Ok(true)
    }
}

#[derive(FromRow)]
pub struct SavedAction {
    #[sqlx(skip)]
    _lease: Option<crate::http::quota_action_owner::ActionLease>,
    action_id: i64,
    requested_by: String,
    requested_at: String,
    finished_at: Option<String>,
    response_status: Option<i64>,
    response_body: Option<String>,
}

impl SavedAction {
    pub fn accepted(&self, reused: bool) -> Response {
        self.respond(StatusCode::ACCEPTED, json!({
            "action_id": self.action_id,
            "message": "Action is queued or running. Follow quota.action on /events and read /quota/actions for its result. Do not retry the operation."
        }).to_string(), reused)
    }
    pub fn respond(&self, status: StatusCode, body: String, reused: bool) -> Response {
        let mut response = Response::builder()
            .status(status)
            .content_type("application/json; charset=utf-8")
            .body(body);
        for (name, value) in [
            ("x-quota-action-id", Some(self.action_id.to_string())),
            ("x-quota-requested-by", Some(self.requested_by.clone())),
            ("x-quota-requested-at", Some(self.requested_at.clone())),
            ("x-quota-finished-at", self.finished_at.clone()),
            ("x-quota-reused", Some(reused.to_string())),
        ] {
            if let Some(value) = value.and_then(|value| HeaderValue::try_from(value).ok()) {
                response.headers_mut().insert(name, value);
            }
        }
        response
    }
}

impl Invocation<'_> {
    pub async fn claim(
        &self,
        state: &AppState,
        api: &QuotaApi,
        kind: &str,
    ) -> Result<Claim, DatabaseError> {
        let Self {
            account_id,
            source_id,
            caller,
            requested_at,
            identity,
            idempotency_key,
        } = *self;
        let before = api.action_snapshot(account_id, source_id).await?;
        let mut write = state.database.write().await?;
        let mut closed = sqlx::query("UPDATE quota_action SET finished_at = ?, outcome = 'interrupted', shared = 0, response_status = 503, response_body = '{\"message\":\"Previous process interrupted action\"}' WHERE finished_at IS NULL AND boot_id IS NOT ?")
            .bind(Timestamp::now().to_string()).bind(state.quota_action_owners.boot()).execute(&mut *write).await?.rows_affected();
        let abandoned = state.quota_action_owners.abandoned();
        for action_id in &abandoned {
            closed += sqlx::query("UPDATE quota_action SET finished_at = ?, outcome = 'interrupted', shared = 0, response_status = 503, response_body = '{\"message\":\"Worker ended before persisting completion\"}' WHERE action_id = ? AND finished_at IS NULL")
                .bind(Timestamp::now().to_string()).bind(action_id).execute(&mut *write).await?.rows_affected();
        }
        if closed > 0 {
            Change::record(&mut write, &Subject::QuotaAction).await?;
        }
        let keyed = if let Some(key) = idempotency_key {
            let scope = sqlx::query_as::<_, (String, Option<String>, Option<String>)>("SELECT kind, account_id, source_id FROM quota_action_key WHERE credential_id = ? AND key = ?")
                .bind(identity.credential_id).bind(key).fetch_optional(&mut *write).await?;
            if scope.as_ref().is_some_and(|(saved_kind, account, source)| {
                saved_kind != kind
                    || account.as_deref() != account_id
                    || source.as_deref() != source_id
            }) {
                return Ok(Claim::Conflict);
            }
            sqlx::query_as::<_, SavedAction>("SELECT action_id, requested_by, requested_at, finished_at, response_status, response_body FROM quota_action WHERE action_id = (SELECT action_id FROM quota_action_key WHERE credential_id = ? AND key = ?)")
                .bind(identity.credential_id).bind(key).fetch_optional(&mut *write).await?
        } else {
            None
        };
        // Compare complete scopes under the SQLite writer lock. The provider-family
        // lock only guards execution; queued requests still need durable action IDs.
        let pending = sqlx::query_as::<_, SavedAction>(
            "SELECT action_id, requested_by, requested_at, finished_at, response_status, response_body \
             FROM quota_action WHERE finished_at IS NULL \
             AND kind = ?1 \
             AND account_id IS ?2 \
             AND source_id IS ?3 ORDER BY action_id DESC LIMIT 1",
        ).bind(kind).bind(account_id).bind(source_id).fetch_optional(&mut *write).await?;
        let saved = sqlx::query_as::<_, SavedAction>(
            "SELECT action_id, requested_by, requested_at, finished_at, response_status, response_body \
             FROM quota_action WHERE kind = ?1 AND source_id IS ?3 \
             AND (account_id IS ?2 OR (?2 IS NOT NULL AND account_id IS NULL AND \
               (finished_at IS NULL OR EXISTS (SELECT 1 FROM json_each(COALESCE(after_state, response_body), '$.accounts') AS a \
                WHERE json_extract(a.value, '$.account.account_id') = ?2 \
                AND (?1 != 'cooldown_reset' OR json_extract(a.value, '$.disabled') = 0) \
                AND (?1 != 'hard_refresh' OR json_extract(a.value, '$.hard_refreshed_at') >= quota_action.requested_at))))) \
             AND (?1 != 'hard_refresh' OR NOT EXISTS (SELECT 1 FROM quota_action AS newer \
             WHERE newer.kind = 'quota_reset' AND newer.action_id > quota_action.action_id \
             AND (?2 IS NULL OR newer.account_id IS ?2))) \
             AND finished_at IS NOT NULL AND shared = 1 AND response_status NOT BETWEEN 400 AND 499 AND finished_at >= ?4 ORDER BY action_id DESC LIMIT 1",
        ).bind(kind).bind(account_id).bind(source_id)
            .bind((requested_at - REUSE_FOR).to_string()).fetch_optional(&mut *write).await?;
        if let Some(saved) = keyed.or(pending).or(saved) {
            if !self.record_key(&mut write, saved.action_id, kind).await? {
                return Ok(Claim::Conflict);
            }
            record_request(&mut write, saved.action_id, caller, requested_at, identity).await?;
            Change::record(&mut write, &Subject::QuotaAction).await?;
            write.commit().await?;
            state.quota_action_owners.forget(&abandoned);
            return Ok(Claim::Reused(saved));
        }
        let action_id = sqlx::query("INSERT INTO quota_action (kind, account_id, source_id, requested_by, requested_at, before_state, shared, credential_id, credential_transport, boot_id) VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, ?)")
            .bind(kind).bind(account_id).bind(source_id).bind(caller)
            .bind(requested_at.to_string()).bind(before).bind(identity.credential_id).bind(identity.transport).bind(state.quota_action_owners.boot()).execute(&mut *write).await?.last_insert_rowid();
        if !self.record_key(&mut write, action_id, kind).await? {
            return Ok(Claim::Conflict);
        }
        record_request(&mut write, action_id, caller, requested_at, identity).await?;
        Change::record(&mut write, &Subject::QuotaAction).await?;
        let lease = state.quota_action_owners.claim(action_id);
        write.commit().await?;
        state.quota_action_owners.forget(&abandoned);
        Ok(Claim::Fresh(SavedAction {
            _lease: Some(lease),
            action_id,
            requested_by: caller.to_owned(),
            requested_at: requested_at.to_string(),
            finished_at: None,
            response_status: None,
            response_body: None,
        }))
    }
}
impl SavedAction {
    pub async fn wait_for_completion(&mut self, state: &AppState) -> Result<(), DatabaseError> {
        let mut changes = state.database.subscribe_to_changes();
        while self.finished_at.is_none() {
            *self = sqlx::query_as::<_, Self>("SELECT action_id, requested_by, requested_at, finished_at, response_status, response_body FROM quota_action WHERE action_id = ?")
                .bind(self.action_id).fetch_one(&state.database.pool).await?;
            if self.finished_at.is_some() || changes.changed().await.is_err() {
                break;
            }
        }
        Ok(())
    }
    pub async fn replay(&self, api: &QuotaApi) -> Result<Response, DatabaseError> {
        if self.finished_at.is_none() {
            return Ok(self.accepted(true));
        }
        let (status, body) = match (self.response_status, &self.response_body) {
            (Some(status), Some(body)) => (u16::try_from(status).ok().and_then(|s| StatusCode::from_u16(s).ok()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), body.clone()),
            _ => (StatusCode::CONFLICT, json!({ "message": "The earlier action is queued or running. Live work is never cancelled by caller timeouts. After a process restart, abandoned records close on the next claim. Cleared reset grants prevent a second spend until Refresh from providers obtains an authoritative grant list." }).to_string()),
        };
        Ok(self.respond(status, current_accounts(api, body).await?, true))
    }
    pub async fn complete(
        &mut self,
        state: &AppState,
        api: &QuotaApi,
        invocation: &Invocation<'_>,
        status: StatusCode,
        body: &str,
    ) {
        let Invocation {
            account_id,
            source_id,
            caller,
            requested_at,
            identity,
            ..
        } = *invocation;
        let action_id = self.action_id;
        let outcome = if body == r#"{"message":"Action interrupted"}"# {
            "interrupted"
        } else {
            outcome(status, body)
        };
        let after = api.action_snapshot(account_id, source_id).await.inspect_err(|error| tracing::error!(%error, "after snapshot failed; retaining operation result")).ok();
        let finished_at = Timestamp::now().to_string();
        let completion: Result<(), DatabaseError> = async {
        let mut write = state.database.write().await?;
        sqlx::query("UPDATE quota_action SET response_status = ?, response_body = ?, outcome = ?, finished_at = ?, shared = CASE WHEN ? = 'interrupted' THEN 0 ELSE shared END WHERE action_id = ?")
            .bind(i64::from(status.as_u16())).bind(body).bind(outcome).bind(&finished_at).bind(outcome).bind(action_id).execute(&mut *write).await?;
        if let Ok(result) = serde_json::from_str::<Value>(body) {
            let mut shared = result["shared_actions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|action| action["action_id"].as_i64())
                .collect::<Vec<_>>();
            shared.sort_unstable();
            shared.dedup();
            for original_id in shared {
                record_request(&mut write, original_id, caller, requested_at, identity).await?;
            }
        }
        Change::record(&mut write, &Subject::QuotaAction).await?;
        write.commit().await?;
            Ok(())
        }.await;
        if let Err(error) = completion {
            tracing::error!(%error, action_id, "operation completed but its result could not be persisted; retain this response and reconcile before another reset");
        }
        // A failed snapshot write must never roll back the already saved HTTP result.
        if let Some(after) = after
            && let Err(error) =
                sqlx::query("UPDATE quota_action SET after_state = ? WHERE action_id = ?")
                    .bind(after)
                    .bind(action_id)
                    .execute(&state.database.pool)
                    .await
        {
            tracing::error!(%error, action_id, "after snapshot could not be persisted; operation result is retained");
        }
        self.finished_at = Some(finished_at);
    }
}

async fn record_request(
    write: &mut crate::database::Write<'_>,
    action_id: i64,
    caller: &str,
    requested_at: Timestamp,
    identity: &CredentialIdentity,
) -> Result<(), DatabaseError> {
    sqlx::query(
        "INSERT INTO quota_action_request (action_id, requested_by, requested_at, credential_id, credential_transport) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(action_id)
    .bind(caller)
    .bind(requested_at.to_string())
    .bind(identity.credential_id).bind(identity.transport)
    .execute(&mut **write)
    .await?;
    Ok(())
}

pub fn error(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .content_type("application/json; charset=utf-8")
        .body(json!({"message": message}).to_string())
}

pub async fn json_response(response: Response) -> (StatusCode, String) {
    let mut status = response.status();
    let body = response
        .into_body()
        .into_string()
        .await
        .unwrap_or_else(|error| {
            status = StatusCode::INTERNAL_SERVER_ERROR;
            json!({"message": error.to_string()}).to_string()
        });
    let body = if serde_json::from_str::<Value>(&body).is_ok() {
        body
    } else {
        json!({"message": body}).to_string()
    };
    (status, body)
}

fn outcome(status: StatusCode, body: &str) -> &'static str {
    if status.is_success() {
        let failed = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|body| body["failed"].as_i64());
        if failed.is_some_and(|failed| failed > 0)
            || serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|b| b["source_failures"].as_array().map(Vec::len))
                .is_some_and(|n| n > 0)
        {
            "partial"
        } else {
            "succeeded"
        }
    } else if status.is_client_error() {
        "rejected"
    } else {
        "failed"
    }
}

pub async fn reject(
    state: &AppState,
    kind: &str,
    identity: &CredentialIdentity,
    caller: &str,
    status: StatusCode,
    message: &str,
) -> Result<Response, DatabaseError> {
    let now = Timestamp::now();
    let body = json!({"message": message}).to_string();
    let mut write = state.database.write().await?;
    let action_id = sqlx::query("INSERT INTO quota_action (kind, requested_by, requested_at, finished_at, outcome, before_state, response_status, response_body, credential_id, credential_transport) VALUES (?, ?, ?, ?, 'rejected', '{}', ?, ?, ?, ?)")
        .bind(kind).bind(caller).bind(now.to_string()).bind(now.to_string()).bind(i64::from(status.as_u16())).bind(&body).bind(identity.credential_id).bind(identity.transport).execute(&mut *write).await?.last_insert_rowid();
    record_request(&mut write, action_id, caller, now, identity).await?;
    Change::record(&mut write, &Subject::QuotaAction).await?;
    write.commit().await?;
    Ok(error(status, message))
}

async fn current_accounts(api: &QuotaApi, body: String) -> Result<String, DatabaseError> {
    let Ok(mut result) = serde_json::from_str::<Value>(&body) else {
        return Ok(body);
    };
    // Results saved by the previous schema still coalesce across an upgrade. Supply
    // the newly added partial-source field without repeating the costly operation.
    if result.get("refreshed").is_some() && result.get("source_failures").is_none() {
        result["source_failures"] = json!([]);
    }
    if result.get("accounts").is_some() {
        let current = api.action_snapshot(None, None).await?;
        if let Ok(current) = serde_json::from_str::<Value>(&current) {
            result["accounts"] = current["accounts"].clone();
        }
    }
    Ok(result.to_string())
}
