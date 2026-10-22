use std::sync::Arc;

use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::account::AccountSummaryOutput;
use crate::http::conditional::conditional;
use crate::prelude::*;
use crate::quota::calibration::{Calibration, Outlier};
use crate::quota::credential::{Cooldown, QuotaAccount};
use crate::quota::manual_reset::{Grant, ResetRequestError};
use crate::quota::prediction::Prediction;
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
    ///
    /// The answer carries an `ETag` of its bytes. Sent back in `If-None-Match`, it answers 304
    /// with no body while those bytes are the same. Every soft sync moves
    /// `soft_observed_at`, and usage moves `last_used_at` and the predictions, so the tag
    /// changes often; follow `/events` to learn when to read again.
    #[oai(
        path = "/quota",
        method = "get",
        operation_id = "list_quota",
        transform = "conditional",
        request_header(
            name = "If-None-Match",
            ty = "Option<String>",
            description = "The ETag of an earlier answer. While the answer is the same, it is 304 with no body."
        ),
        response_header(
            name = "ETag",
            ty = "Option<String>",
            description = "A tag of the answer's bytes, to send back in If-None-Match. Absent on an error."
        )
    )]
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

    /// Spends one manual reset the provider granted the account, then clears CLIProxy's
    /// cooldowns on it and hard refreshes it, as the official management UI does. Only an
    /// explicit `confirmed: true` spends one, and a failed request is never retried here.
    /// A credential CLIProxy no longer lists, a disabled one, a provider whose resets cannot
    /// be spent here and an account without an unexpired one are refused before the provider
    /// is asked. Once the provider confirmed the reset the answer is 200: `failed` is 1 when
    /// the quota could not be refreshed afterwards, with the reason in `hard_refresh_error`,
    /// and `manual_resets` stays empty until a provider list is read successfully. Stored
    /// resets are cleared before the consuming request, so an unknown outcome also blocks
    /// another reset across clients and restarts until a hard refresh reads that list.
    #[oai(path = "/quota/reset", method = "post", operation_id = "reset_quota")]
    async fn reset_quota(&self, input: Json<QuotaResetInput>) -> QuotaRefreshResponse {
        if !input.0.confirmed {
            return QuotaRefreshResponse::Invalid(Json(message(
                "a manual reset is spent only when confirmed is true",
            )));
        }
        let Ok(account_id) = input.0.account_id.parse::<Id<Account>>() else {
            return QuotaRefreshResponse::Missing(Json(message("account was not found")));
        };
        let reset = match crate::quota::manual_reset::spend(
            &self.state,
            &self.state.http,
            account_id,
        )
        .await
        {
            Ok(reset) => reset,
            Err(ResetRequestError::UnknownAccount) => {
                return QuotaRefreshResponse::Missing(Json(message("account was not found")));
            }
            Err(
                error @ (ResetRequestError::NotListed
                | ResetRequestError::Disabled
                | ResetRequestError::NotResettable
                | ResetRequestError::NoneAvailable),
            ) => return QuotaRefreshResponse::Invalid(Json(message(&error.to_string()))),
            Err(
                error @ (ResetRequestError::Unreachable(_) | ResetRequestError::OutcomeUnknown(_)),
            ) => {
                return QuotaRefreshResponse::Unreachable(Json(message(&error.to_string())));
            }
            Err(ResetRequestError::Database(error)) => {
                return QuotaRefreshResponse::Failed(Json(failure("reset_quota", &error)));
            }
        };
        match self.everything("reset_quota").await {
            Ok(list) => QuotaRefreshResponse::Refreshed(Json(QuotaRefresh {
                refreshed: reset.refreshed,
                failed: reset.failed,
                accounts: list.accounts,
            })),
            Err(error) => QuotaRefreshResponse::Failed(Json(error)),
        }
    }
}

/// A provider-reported limit, from a hard refresh or a normal request's response.
///
/// `meta-subscription` records an exhausted Meta subscription and its reset from a 429,
/// without inventing an amount or a window duration. A later success or the named reset
/// removes it and records `quota.updated`, so consumers read this list again.
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
    /// What the window holds now, from the report and the usage this service metered since.
    /// Absent when nothing was metered since a report that still holds, or when nothing
    /// relates metered usage to the window.
    prediction: Option<QuotaPredictionOutput>,
    /// How much of the window one list dollar spends, learned from the windows reported
    /// over the last four weeks.
    calibration: Option<QuotaCalibrationOutput>,
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
            prediction: window.prediction.map(QuotaPredictionOutput::from),
            calibration: window.calibration.map(QuotaCalibrationOutput::from),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "QuotaPrediction", skip_serializing_if_is_none)]
pub struct QuotaPredictionOutput {
    /// Share of the window used by now, 0 to 1.
    used_fraction: f64,
    /// When the window that holds now resets. Absent for a window that opens on its next
    /// request and has had none since it reset.
    resets_at: Option<String>,
    /// The window reset after the report, so the report no longer describes it.
    has_reset: bool,
}

impl From<Prediction> for QuotaPredictionOutput {
    fn from(prediction: Prediction) -> Self {
        Self {
            used_fraction: prediction.used_fraction,
            resets_at: prediction.resets_at.map(|at| at.to_string()),
            has_reset: prediction.has_reset,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "QuotaCalibration")]
pub struct QuotaCalibrationOutput {
    /// The median share of the window one list dollar spent.
    fraction_per_usd: f64,
    /// The windows the median was taken over.
    windows: u32,
    /// Windows whose share per dollar is off the median by more than a factor of two, such
    /// as one the provider reset early. Newest first.
    outliers: Vec<QuotaOutlierOutput>,
}

impl From<Calibration> for QuotaCalibrationOutput {
    fn from(calibration: Calibration) -> Self {
        Self {
            fraction_per_usd: calibration.fraction_per_usd,
            windows: u32::try_from(calibration.windows).unwrap_or(u32::MAX),
            outliers: calibration
                .outliers
                .into_iter()
                .map(QuotaOutlierOutput::from)
                .collect(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "QuotaOutlier")]
pub struct QuotaOutlierOutput {
    started_at: String,
    fraction_per_usd: f64,
}

impl From<Outlier> for QuotaOutlierOutput {
    fn from(outlier: Outlier) -> Self {
        Self {
            started_at: outlier.started_at.to_string(),
            fraction_per_usd: outlier.fraction_per_usd,
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

/// A manual quota reset grant the provider reported, with at least one reset left. A Codex
/// credit is one reset with no label or total.
#[derive(Debug, Object)]
#[oai(rename = "ManualReset", skip_serializing_if_is_none)]
pub struct ManualResetOutput {
    /// The provider's name for the grant, such as a promotion.
    label: Option<String>,
    /// Resets left in the grant; at least 1.
    remaining: i64,
    /// Resets the grant started with, when the provider states it.
    total: Option<i64>,
    /// RFC 3339. The grant expires at this time. Absent if the provider gives no expiry date.
    expires_at: Option<String>,
    /// Whether the provider's grants can be spent through `POST /quota/reset`. Claude grants
    /// are listed but cannot be spent here, and a disabled or removed credential is refused.
    spendable: bool,
}

impl ManualResetOutput {
    fn new(grant: Grant, spendable: bool) -> Self {
        Self {
            label: grant.label,
            remaining: grant.remaining,
            total: grant.total,
            expires_at: grant.expires_at.map(|at| at.0.to_string()),
            spendable,
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
    /// When CLIProxy's credential list first left the credential out. Present while CLIProxy
    /// no longer lists it.
    removed_at: Option<String>,
    /// When the last request charged to the credential happened.
    last_used_at: Option<String>,
    windows: Vec<QuotaWindowOutput>,
    /// Every manual reset grant that has not expired, soonest expiry first. Grants with no
    /// expiry date come last. Empty if the provider grants none or its list is not known, and
    /// after a reset is spent until a hard refresh reads the list from the provider again.
    manual_resets: Vec<ManualResetOutput>,
}

impl From<QuotaAccount> for QuotaAccountOutput {
    fn from(account: QuotaAccount) -> Self {
        let spendable = account.spends_manual_resets();
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
            removed_at: account.removed_at.map(|at| at.0.to_string()),
            last_used_at: account.last_used_at.map(|at| at.0.to_string()),
            windows: account.windows.into_iter().map(Into::into).collect(),
            manual_resets: account
                .manual_resets
                .into_iter()
                .map(|grant| ManualResetOutput::new(grant, spendable))
                .collect(),
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

#[derive(Debug, Object)]
pub struct QuotaResetInput {
    account_id: String,
    /// Must be true: the reset is spent at the provider and cannot be taken back.
    confirmed: bool,
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
