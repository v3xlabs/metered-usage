use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object};

use crate::http::api::Error;
use crate::http::api::account::AccountSummaryOutput;
use crate::quota::calibration::{Calibration, Outlier};
use crate::quota::credential::{Cooldown, QuotaAccount};
use crate::quota::manual_reset::Grant;
use crate::quota::prediction::Prediction;
use crate::quota::sign_in::{Reauthentication, SignInError};
use crate::quota::window::QuotaWindow;

/// A provider-reported limit, from a hard refresh or a normal request's response.
///
/// `meta-subscription` records an exhausted Meta subscription and its reset from a 429,
/// without inventing an amount or a window duration. A later success or the named reset
/// removes it and records `quota.updated`, so consumers read this list again.
#[derive(Debug, Object)]
#[oai(rename = "QuotaWindow", skip_serializing_if_is_none)]
pub struct QuotaWindowOutput {
    pub(super) window_key: String,
    pub(super) label: String,
    /// Share of the window used, 0 to 1, whatever unit the provider counts in.
    pub(super) used_fraction: Option<f64>,
    pub(super) used_value: Option<f64>,
    pub(super) limit_value: Option<f64>,
    /// What `used_value` and `limit_value` count.
    pub(super) unit: Option<String>,
    pub(super) window_seconds: Option<i64>,
    /// When the provider said the window resets.
    pub(super) resets_at: Option<String>,
    pub(super) observed_at: String,
    /// What the window holds now, from the report and the usage this service metered since.
    /// Absent when nothing was metered since a report that still holds, or when nothing
    /// relates metered usage to the window.
    pub(super) prediction: Option<QuotaPredictionOutput>,
    /// How much of the window one list dollar spends, learned from the windows reported
    /// over the last four weeks.
    pub(super) calibration: Option<QuotaCalibrationOutput>,
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
    pub(super) used_fraction: f64,
    /// When the window that holds now resets. Absent for a window that opens on its next
    /// request and has had none since it reset.
    pub(super) resets_at: Option<String>,
    /// The window reset after the report, so the report no longer describes it.
    pub(super) has_reset: bool,
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
    pub(super) fraction_per_usd: f64,
    /// The windows the median was taken over.
    pub(super) windows: u32,
    /// Windows whose share per dollar is off the median by more than a factor of two, such
    /// as one the provider reset early. Newest first.
    pub(super) outliers: Vec<QuotaOutlierOutput>,
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
    pub(super) started_at: String,
    pub(super) fraction_per_usd: f64,
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
    pub(super) scope: String,
    pub(super) model_key: Option<String>,
    pub(super) reason: String,
    pub(super) retry_at: String,
    /// Seconds left when CLIProxy was last read.
    pub(super) remaining_seconds: i64,
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
    pub(super) label: Option<String>,
    /// Resets left in the grant; at least 1.
    pub(super) remaining: i64,
    /// Resets the grant started with, when the provider states it.
    pub(super) total: Option<i64>,
    /// RFC 3339. The grant expires at this time. Absent if the provider gives no expiry date.
    pub(super) expires_at: Option<String>,
    /// Whether the provider's grants can be spent through `POST /quota/reset`. Claude grants
    /// are listed but cannot be spent here, and a disabled or removed credential is refused.
    pub(super) spendable: bool,
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
    pub(super) account: AccountSummaryOutput,
    pub(super) status: Option<String>,
    pub(super) status_message: Option<String>,
    pub(super) disabled: bool,
    pub(super) unavailable: bool,
    /// CLIProxy's scheduling decision: the instant before which CLIProxy will not route
    /// requests to this credential. It is not the provider's quota reset time; that is
    /// `resets_at` on a window.
    pub(super) next_retry_after: Option<String>,
    pub(super) cooldowns: Vec<CooldownOutput>,
    pub(super) account_type: Option<String>,
    pub(super) plan: Option<String>,
    /// When CLIProxy's credential list last described this credential.
    pub(super) soft_observed_at: Option<String>,
    /// When a hard refresh last ran for this credential, successful or not.
    pub(super) hard_refreshed_at: Option<String>,
    /// Why the last hard refresh failed; absent after a successful one.
    pub(super) hard_refresh_error: Option<String>,
    /// An explicit 401 expired-token answer. A later successful quota read clears it; the
    /// user must refresh after signing in again. CLIProxy credentials it no
    /// longer lists keep it, so check `removed_at`. Read again after `account.changed`; window
    /// ages remain in `windows[].observed_at`.
    pub(super) sign_in_error: Option<SignInError>,
    /// Absent unless sign-in expired and this provider has a verified management UI action.
    pub(super) reauthentication: Option<Reauthentication>,
    /// Whether the provider has an adapter and the credential carries what it needs.
    pub(super) hard_refreshable: bool,
    /// When CLIProxy's credential list first left the credential out. Present while CLIProxy
    /// no longer lists it.
    pub(super) removed_at: Option<String>,
    /// When the last request charged to the credential happened.
    pub(super) last_used_at: Option<String>,
    pub(super) windows: Vec<QuotaWindowOutput>,
    /// Every manual reset grant that has not expired, soonest expiry first. Grants with no
    /// expiry date come last. Empty if the provider grants none or its list is not known, and
    /// after a reset is spent until a hard refresh reads the list from the provider again.
    pub(super) manual_resets: Vec<ManualResetOutput>,
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
            sign_in_error: account.sign_in_error,
            reauthentication: account.reauthentication,
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
    pub(super) accounts: Vec<QuotaAccountOutput>,
}

impl From<Vec<QuotaAccount>> for QuotaList {
    fn from(accounts: Vec<QuotaAccount>) -> Self {
        Self {
            accounts: accounts.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Object)]
pub struct QuotaSourceFailure {
    source: String,
    message: String,
}

impl From<crate::quota::SourceFailure> for QuotaSourceFailure {
    fn from(failure: crate::quota::SourceFailure) -> Self {
        Self {
            source: failure.source,
            message: failure.error.to_string(),
        }
    }
}

#[derive(Debug, Object)]
pub struct QuotaRefresh {
    pub(super) refreshed: i64,
    pub(super) failed: i64,
    /// Sources that were unreachable. Successful sources and counts are retained.
    pub(super) source_failures: Vec<QuotaSourceFailure>,
    /// Earlier per-account operations reused by a bulk operation, or bulk operations reused by an account request.
    pub(super) shared_actions: Vec<crate::quota::reuse::SharedQuotaAction>,
    pub(super) accounts: Vec<QuotaAccountOutput>,
}

#[derive(Debug, Object)]
pub struct QuotaSyncInput {
    pub(super) source_id: Option<String>,
}

#[derive(Debug, Object)]
pub struct QuotaRefreshInput {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Object)]
pub struct QuotaCooldownResetInput {
    /// The account whose CLIProxy scheduling cooldowns should be cleared. Required.
    pub(super) account_id: String,
}

#[derive(Debug, Object)]
pub struct QuotaResetInput {
    pub(super) account_id: String,
    /// Must be true: the reset is spent at the provider and cannot be taken back.
    pub(super) confirmed: bool,
}

#[derive(ApiResponse)]
pub(super) enum QuotaListResponse {
    #[oai(status = 200)]
    Found(Json<QuotaList>),
    #[expect(dead_code, reason = "the shared-action transform emits this response")]
    #[oai(status = 202)]
    Pending(Json<QuotaActionPending>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

/// The HTTP wait ended; the action is queued or running and publishes its final result in history.
#[derive(Debug, Object)]
#[oai(rename = "QuotaActionPending")]
pub(super) struct QuotaActionPending {
    /// Read /quota/actions after `action_id` - 1 and follow quota.action on /events.
    action_id: i64,
    message: String,
}

#[derive(ApiResponse)]
pub(super) enum QuotaSyncResponse {
    #[oai(status = 200)]
    Synced(Json<QuotaList>),
    #[expect(dead_code, reason = "the shared-action transform emits this response")]
    #[oai(status = 202)]
    Pending(Json<QuotaActionPending>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[expect(
        dead_code,
        reason = "the shared-action transform emits this documented response before the handler"
    )]
    #[oai(status = 409)]
    Unknown(Json<Error>),
    #[oai(status = 502)]
    Unreachable(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
pub(super) enum QuotaRefreshResponse {
    #[oai(status = 200)]
    Refreshed(Json<QuotaRefresh>),
    #[expect(dead_code, reason = "the shared-action transform emits this response")]
    #[oai(status = 202)]
    Pending(Json<QuotaActionPending>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[expect(
        dead_code,
        reason = "the shared-action transform emits this documented response before the handler"
    )]
    #[oai(status = 409)]
    Unknown(Json<Error>),
    #[oai(status = 502)]
    Unreachable(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}
