//! Durable records of human actions and automatic recovery. Snapshots contain public quota data, never tokens.

use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object, OpenApi, types::ToJSON};
use sqlx::FromRow;

use crate::http::api::Error;
use crate::http::api::quota::QuotaApi;
use crate::http::api::quota_output::QuotaList;
use crate::prelude::*;
use crate::quota::credential::QuotaAccount;

/// A page bounds response size since snapshots can contain many accounts.
const ACTION_PAGE: i64 = 50;

#[derive(Debug, Object, FromRow)]
#[oai(rename = "QuotaAction", skip_serializing_if_is_none)]
pub struct QuotaActionOutput {
    action_id: i64,
    /// `soft_refresh`, `hard_refresh`, `cooldown_reset` or `quota_reset`.
    kind: String,
    account_id: Option<String>,
    source_id: Option<String>,
    requested_at: String,
    requested_by: String,
    credential_id: String,
    credential_transport: String,
    #[sqlx(skip)]
    requests: Vec<ActionRequestOutput>,
    finished_at: Option<String>,
    /// `succeeded`, `partial`, `rejected`, `failed`, `interrupted` or `reconciled_unknown`.
    /// Absent while running, or if the process stopped before recording an outcome.
    /// A failed or unfinished reset may have reached the provider. Never retry blindly.
    outcome: Option<String>,
    /// JSON encoded `QuotaList`, limited to the requested scope, before the operation.
    before_state: String,
    /// JSON encoded `QuotaList` after the operation, including partial observations on failure.
    after_state: Option<String>,
    /// Saved HTTP result, including shared-action attribution and failures.
    response_status: Option<i64>,
    response_body: Option<String>,
}

#[derive(Debug, Object)]
#[oai(rename = "QuotaActionRequest")]
pub struct ActionRequestOutput {
    requested_by: String,
    requested_at: String,
    credential_id: String,
    credential_transport: String,
}

pub struct ActionApi {
    pub state: std::sync::Arc<crate::app::AppState>,
}

#[OpenApi]
impl ActionApi {
    /// Durable human and automatic recovery quota action history, oldest first, at most 50 records after `after`.
    /// Follow `quota.action` on /events, then reread this history. Running records can change:
    /// retain their `action_id` and reread from just before it until `finished_at` is present.
    /// An unfinished record after a restart has unknown outcome. Do not retry a quota reset.
    /// No provider or gateway request is made by this read.
    #[oai(
        path = "/quota/actions",
        method = "get",
        operation_id = "list_quota_actions"
    )]
    async fn list(&self, after: Query<Option<i64>>) -> ActionResponse {
        match sqlx::query_as::<_, QuotaActionOutput>(
            "SELECT action_id, kind, account_id, source_id, requested_at, requested_by, credential_id, credential_transport, finished_at, \
             outcome, before_state, after_state, response_status, response_body \
             FROM quota_action WHERE action_id > ? ORDER BY action_id LIMIT ?",
        )
        .bind(after.0.unwrap_or(0))
        .bind(ACTION_PAGE)
        .fetch_all(&self.state.database.pool)
        .await
        {
            Ok(mut actions) => {
                let requests = sqlx::query_as::<_, (i64, String, String, String, String)>(
                    "SELECT action_id, requested_by, requested_at, credential_id, credential_transport FROM quota_action_request WHERE action_id > ? AND action_id <= ? ORDER BY request_id",
                ).bind(after.0.unwrap_or(0)).bind(actions.last().map_or(0, |action| action.action_id))
                    .fetch_all(&self.state.database.pool).await;
                let requests = match requests {
                    Ok(requests) => requests,
                    Err(error) => {
                        tracing::error!(%error, "quota action requests could not be read");
                        return ActionResponse::Failed(Json(Error { message: "the database refused the request".to_owned() }));
                    }
                };
                for action in &mut actions {
                    action.requests = requests.iter().filter(|(action_id, ..)| *action_id == action.action_id)
                        .map(|(_, requested_by, requested_at, credential_id, credential_transport)| ActionRequestOutput { requested_by: requested_by.clone(), requested_at: requested_at.clone(), credential_id: credential_id.clone(), credential_transport: credential_transport.clone() }).collect();
                }
                ActionResponse::Found(Json(actions))
            },
            Err(error) => {
                tracing::error!(%error, "quota action history could not be read");
                ActionResponse::Failed(Json(Error {
                    message: "the database refused the request".to_owned(),
                }))
            }
        }
    }
}

impl QuotaApi {
    pub async fn action_snapshot(
        &self,
        account_id: Option<&str>,
        source_id: Option<&str>,
    ) -> Result<String, DatabaseError> {
        let accounts = QuotaAccount::list(&self.state.database, None)
            .await?
            .into_iter()
            .filter(|account| {
                account_id.is_none_or(|id| account.account.id.encode() == id)
                    && source_id.is_none_or(|id| account.account.source_id.encode() == id)
            })
            .collect::<Vec<_>>();
        Ok(serde_json::Value::from(QuotaList::from(accounts).to_json()).to_string())
    }
}

#[derive(ApiResponse)]
enum ActionResponse {
    #[oai(status = 200)]
    Found(Json<Vec<QuotaActionOutput>>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}
