use std::sync::Arc;

use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::account::AccountSummaryOutput;
use crate::prelude::*;
use crate::quota::credential::{Cooldown, QuotaAccount};
use crate::quota::window::QuotaWindow;
use crate::quota::{RefreshRequestError, SourceFailure};

pub struct QuotaApi {
    pub state: Arc<AppState>,
}

impl QuotaApi {
    #[must_use]
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    async fn everything(&self, operation: &'static str) -> Result<QuotaList, Error> {
        QuotaAccount::list(&self.state.database, None)
            .await
            .map(QuotaList::from)
            .map_err(|error| failure(operation, &error))
    }
}

#[OpenApi]
impl QuotaApi {
    /// The last known state and quota of every CLIProxy credential, from the database only.
    #[oai(path = "/quota", method = "get", operation_id = "list_quota")]
    async fn list_quota(&self, source_id: Query<Option<String>>) -> QuotaListResponse {
        let source_id = match source_id.0.as_deref().map(str::parse::<Id<Source>>) {
            None => None,
            Some(Ok(id)) => Some(id),
            Some(Err(_)) => {
                return QuotaListResponse::Invalid(Json(message("source_id is not an id")));
            }
        };
        match QuotaAccount::list(&self.state.database, source_id).await {
            Ok(accounts) => QuotaListResponse::Found(Json(accounts.into())),
            Err(error) => QuotaListResponse::Failed(Json(failure("list_quota", &error))),
        }
    }

    /// Reads CLIProxy's credential list now, which reaches no provider, and answers every
    /// credential. 502 names each source that could not be read; the others are stored.
    #[oai(path = "/quota/sync", method = "post", operation_id = "sync_quota")]
    async fn sync_quota(&self, input: Json<QuotaSyncInput>) -> QuotaSyncResponse {
        let only = match input.0.source_id.as_deref().map(str::parse::<Id<Source>>) {
            None => None,
            Some(Ok(id)) => match Source::load(&self.state.database, id).await {
                Ok(Some(source)) if source.kind == SourceKind::CliProxy => Some(id),
                Ok(Some(_)) => {
                    return QuotaSyncResponse::Invalid(Json(message(
                        "the source is not a CLIProxy source",
                    )));
                }
                Ok(None) => {
                    return QuotaSyncResponse::Missing(Json(message("source was not found")));
                }
                Err(error) => {
                    return QuotaSyncResponse::Failed(Json(failure("sync_quota", &error)));
                }
            },
            Some(Err(_)) => {
                return QuotaSyncResponse::Missing(Json(message("source was not found")));
            }
        };
        let failures = match crate::quota::sync(&self.state, &self.state.http, only).await {
            Ok(failures) => failures,
            Err(error) => return QuotaSyncResponse::Failed(Json(failure("sync_quota", &error))),
        };
        if !failures.is_empty() {
            return QuotaSyncResponse::Unreachable(Json(unreachable(&failures)));
        }
        match self.everything("sync_quota").await {
            Ok(list) => QuotaSyncResponse::Synced(Json(list)),
            Err(error) => QuotaSyncResponse::Failed(Json(error)),
        }
    }

    /// Asks each provider for its quota through CLIProxy, as the official management UI
    /// does, and answers every credential. Without `account_id` every hard-refreshable
    /// credential is refreshed. A credential whose provider fails records the reason in
    /// `hard_refresh_error` and counts in `failed`; it does not fail the request.
    #[oai(
        path = "/quota/refresh",
        method = "post",
        operation_id = "refresh_quota"
    )]
    async fn refresh_quota(&self, input: Json<QuotaRefreshInput>) -> QuotaRefreshResponse {
        let only = match input.0.account_id.as_deref().map(str::parse::<Id<Account>>) {
            None => None,
            Some(Ok(id)) => Some(id),
            Some(Err(_)) => {
                return QuotaRefreshResponse::Missing(Json(message("account was not found")));
            }
        };
        let refresh = match crate::quota::refresh(&self.state, &self.state.http, only).await {
            Ok(refresh) => refresh,
            Err(RefreshRequestError::UnknownAccount) => {
                return QuotaRefreshResponse::Missing(Json(message("account was not found")));
            }
            Err(error @ RefreshRequestError::NotRefreshable) => {
                return QuotaRefreshResponse::Invalid(Json(message(&error.to_string())));
            }
            Err(error @ RefreshRequestError::Unreachable(_)) => {
                return QuotaRefreshResponse::Unreachable(Json(message(&error.to_string())));
            }
            Err(RefreshRequestError::Database(error)) => {
                return QuotaRefreshResponse::Failed(Json(failure("refresh_quota", &error)));
            }
        };
        if !refresh.unreachable.is_empty() {
            return QuotaRefreshResponse::Unreachable(Json(unreachable(&refresh.unreachable)));
        }
        match self.everything("refresh_quota").await {
            Ok(list) => QuotaRefreshResponse::Refreshed(Json(QuotaRefresh {
                refreshed: refresh.refreshed,
                failed: refresh.failed,
                accounts: list.accounts,
            })),
            Err(error) => QuotaRefreshResponse::Failed(Json(error)),
        }
    }
}

/// One limit the provider reported at the last successful hard refresh.
#[derive(Debug, Object)]
#[oai(rename = "QuotaWindow", skip_serializing_if_is_none)]
pub struct QuotaWindowOutput {
    window_key: String,
    label: String,
    /// Share of the window used, 0 to 1, whatever unit the provider counts in.
    used_fraction: Option<f64>,
    used_value: Option<f64>,
    limit_value: Option<f64>,
    /// What `used_value` and `limit_value` count.
    unit: Option<String>,
    window_seconds: Option<i64>,
    /// When the provider said the window resets.
    resets_at: Option<String>,
    observed_at: String,
}

impl From<QuotaWindow> for QuotaWindowOutput {
    fn from(window: QuotaWindow) -> Self {
        Self {
            window_key: window.window_key,
            label: window.label,
            used_fraction: window.used_fraction,
            used_value: window.used_value,
            limit_value: window.limit_value,
            unit: window.unit,
            window_seconds: window.window_seconds,
            resets_at: window.resets_at.map(|at| at.0.to_string()),
            observed_at: window.observed_at.to_string(),
        }
    }
}

/// A CLIProxy cooldown timer on the credential or on one of its models.
#[derive(Debug, Object)]
#[oai(rename = "Cooldown", skip_serializing_if_is_none)]
pub struct CooldownOutput {
    scope: String,
    model_key: Option<String>,
    reason: String,
    retry_at: String,
    /// Seconds left when CLIProxy was last read.
    remaining_seconds: i64,
}

impl From<Cooldown> for CooldownOutput {
    fn from(cooldown: Cooldown) -> Self {
        Self {
            scope: cooldown.scope,
            model_key: cooldown.model_key,
            reason: cooldown.reason,
            retry_at: cooldown.retry_at.to_string(),
            remaining_seconds: cooldown.remaining_seconds,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "QuotaAccount", skip_serializing_if_is_none)]
pub struct QuotaAccountOutput {
    account: AccountSummaryOutput,
    status: Option<String>,
    status_message: Option<String>,
    disabled: bool,
    unavailable: bool,
    /// CLIProxy's scheduling decision: the instant before which CLIProxy will not route
    /// requests to this credential. It is not the provider's quota reset time; that is
    /// `resets_at` on a window.
    next_retry_after: Option<String>,
    cooldowns: Vec<CooldownOutput>,
    account_type: Option<String>,
    plan: Option<String>,
    /// When CLIProxy's credential list last described this credential.
    soft_observed_at: Option<String>,
    /// When a hard refresh last ran for this credential, successful or not.
    hard_refreshed_at: Option<String>,
    /// Why the last hard refresh failed; absent after a successful one.
    hard_refresh_error: Option<String>,
    /// Whether the provider has an adapter and the credential carries what it needs.
    hard_refreshable: bool,
    windows: Vec<QuotaWindowOutput>,
}

impl From<QuotaAccount> for QuotaAccountOutput {
    fn from(account: QuotaAccount) -> Self {
        Self {
            account: account.account.into(),
            status: account.status,
            status_message: account.status_message,
            disabled: account.disabled,
            unavailable: account.unavailable,
            next_retry_after: account.next_retry_after.map(|at| at.0.to_string()),
            cooldowns: account.cooldowns.0.into_iter().map(Into::into).collect(),
            account_type: account.account_type,
            plan: account.plan,
            soft_observed_at: account.soft_observed_at.map(|at| at.0.to_string()),
            hard_refreshed_at: account.hard_refreshed_at.map(|at| at.0.to_string()),
            hard_refresh_error: account.hard_refresh_error,
            hard_refreshable: account.hard_refreshable,
            windows: account.windows.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Object)]
pub struct QuotaList {
    accounts: Vec<QuotaAccountOutput>,
}

impl From<Vec<QuotaAccount>> for QuotaList {
    fn from(accounts: Vec<QuotaAccount>) -> Self {
        Self {
            accounts: accounts.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Object)]
pub struct QuotaRefresh {
    refreshed: i64,
    failed: i64,
    accounts: Vec<QuotaAccountOutput>,
}

#[derive(Debug, Object)]
pub struct QuotaSyncInput {
    source_id: Option<String>,
}

#[derive(Debug, Object)]
pub struct QuotaRefreshInput {
    account_id: Option<String>,
}

#[derive(ApiResponse)]
enum QuotaListResponse {
    #[oai(status = 200)]
    Found(Json<QuotaList>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum QuotaSyncResponse {
    #[oai(status = 200)]
    Synced(Json<QuotaList>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 502)]
    Unreachable(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum QuotaRefreshResponse {
    #[oai(status = 200)]
    Refreshed(Json<QuotaRefresh>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 502)]
    Unreachable(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn message(text: &str) -> Error {
    Error {
        message: text.to_owned(),
    }
}

fn unreachable(failures: &[SourceFailure]) -> Error {
    Error {
        message: failures
            .iter()
            .map(|failure| format!("source {}: {}", failure.source, failure.error))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn failure(operation: &'static str, error: &DatabaseError) -> Error {
    tracing::error!(%operation, %error, "quota operation failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
