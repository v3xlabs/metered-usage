use std::sync::Arc;

use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};

use crate::account::{Credential, MergeError};
use crate::app::AppState;
use crate::http::api::Error;
use crate::http::conditional::conditional;
use crate::prelude::*;
use crate::quota::sign_in::{Reauthentication, SignIn, SignInError};

pub struct AccountApi {
    pub state: Arc<AppState>,
}

impl AccountApi {
    async fn output(&self, account: Account) -> Result<AccountOutput, DatabaseError> {
        let sign_in = SignIn::list(&self.state.database, Some(account.summary.id))
            .await?
            .remove(&account.summary.id)
            .unwrap_or_default();
        Ok((account, sign_in).into())
    }
}

#[OpenApi]
impl AccountApi {
    /// The answer carries an `ETag` of its bytes. Sent back in `If-None-Match`, it answers 304
    /// with no body while those bytes are the same. Usage moves `last_seen_at`,
    /// `last_used_at` and `event_count`, so the tag changes often; follow `/events` to learn
    /// when to read again.
    /// `plan_continuation_account_ids` suggests replacements for a removed credential,
    /// never an identity. Read again after `account.changed` or `plan.changed`. `credentials`
    /// lists previous sign-ins explicitly merged into the account.
    /// `sign_in_error` and `reauthentication` match `/quota`; `account.changed` announces
    /// expiry and recovery. Read `/quota` for retained window values and their `observed_at`.
    #[oai(
        path = "/accounts",
        method = "get",
        operation_id = "list_accounts",
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
    async fn list_accounts(&self) -> AccountListResponse {
        let mut sign_ins = match SignIn::list(&self.state.database, None).await {
            Ok(sign_ins) => sign_ins,
            Err(error) => {
                return AccountListResponse::Failed(Json(failure("list_accounts", &error)));
            }
        };
        match Account::list(&self.state.database).await {
            Ok(accounts) => AccountListResponse::Found(Json(AccountList {
                accounts: accounts
                    .into_iter()
                    .map(|account| {
                        let sign_in = sign_ins.remove(&account.summary.id).unwrap_or_default();
                        (account, sign_in).into()
                    })
                    .collect(),
            })),
            Err(error) => AccountListResponse::Failed(Json(failure("list_accounts", &error))),
        }
    }

    /// Names an account for people. An absent, null or blank name clears it.
    #[oai(
        path = "/accounts/:account_id",
        method = "patch",
        operation_id = "update_account"
    )]
    async fn update_account(
        &self,
        account_id: Path<String>,
        input: Json<AccountUpdate>,
    ) -> AccountResponse {
        let Ok(account_id) = account_id.0.parse::<Id<Account>>() else {
            return AccountResponse::Missing(Json(missing()));
        };
        let display_name = input
            .0
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        match Account::rename(&self.state.database, account_id, display_name).await {
            Ok(Some(account)) => match self.output(account).await {
                Ok(output) => AccountResponse::Found(Json(Box::new(output))),
                Err(error) => AccountResponse::Failed(Json(failure("update_account", &error))),
            },
            Ok(None) => AccountResponse::Missing(Json(missing())),
            Err(error) => AccountResponse::Failed(Json(failure("update_account", &error))),
        }
    }

    /// Folds an account into another of the same source: its events move to the target,
    /// and any record that later names its credential is charged to the target.
    /// With `require_plan_continuation`, rechecks the suggested pair before moving history.
    #[oai(
        path = "/accounts/:account_id/merge",
        method = "post",
        operation_id = "merge_account"
    )]
    async fn merge_account(
        &self,
        account_id: Path<String>,
        input: Json<MergeInput>,
    ) -> MergeResponse {
        let (Ok(account_id), Ok(into)) = (
            account_id.0.parse::<Id<Account>>(),
            input.0.into_account_id.parse::<Id<Account>>(),
        ) else {
            return MergeResponse::Missing(Json(missing()));
        };
        match Account::merge(
            &self.state.database,
            account_id,
            into,
            input.0.require_plan_continuation.unwrap_or(false),
        )
        .await
        {
            Ok(account) => match self.output(account).await {
                Ok(output) => MergeResponse::Merged(Json(Box::new(output))),
                Err(error) => MergeResponse::Failed(Json(failure("merge_account", &error))),
            },
            Err(MergeError::Missing) => MergeResponse::Missing(Json(missing())),
            Err(
                error @ (MergeError::Same
                | MergeError::DifferentSources
                | MergeError::SourceMerged
                | MergeError::TargetMerged
                | MergeError::ContinuationChanged),
            ) => MergeResponse::Invalid(Json(Error {
                message: error.to_string(),
            })),
            Err(MergeError::Database(error)) => {
                MergeResponse::Failed(Json(failure("merge_account", &error)))
            }
        }
    }

    /// Remembers that two credentials belong to different plans. Repeating the decision
    /// records no change. The pair remains dismissed when either credential reappears.
    #[oai(
        path = "/accounts/:account_id/different-plan",
        method = "post",
        operation_id = "dismiss_plan_continuation"
    )]
    async fn dismiss_plan_continuation(
        &self,
        account_id: Path<String>,
        input: Json<ContinuationDismissInput>,
    ) -> DismissResponse {
        let (Ok(account_id), Ok(other)) = (
            account_id.0.parse::<Id<Account>>(),
            input.0.other_account_id.parse::<Id<Account>>(),
        ) else {
            return DismissResponse::Missing(Json(missing()));
        };
        match Account::dismiss_continuation(&self.state.database, account_id, other).await {
            Ok(true) => DismissResponse::Dismissed,
            Ok(false) => DismissResponse::Missing(Json(missing())),
            Err(error) => {
                DismissResponse::Failed(Json(failure("dismiss_plan_continuation", &error)))
            }
        }
    }
}

#[derive(Debug, Clone, Object)]
#[oai(rename = "AccountSummary", skip_serializing_if_is_none)]
pub struct AccountSummaryOutput {
    account_id: String,
    source_id: String,
    source_name: String,
    provider: String,
    auth_kind: AuthKindOutput,
    label: Option<String>,
    display_name: Option<String>,
}

impl From<AccountSummary> for AccountSummaryOutput {
    fn from(summary: AccountSummary) -> Self {
        Self {
            account_id: summary.id.encode(),
            source_id: summary.source_id.encode(),
            source_name: summary.source_name,
            provider: summary.provider,
            auth_kind: summary.auth_kind.into(),
            label: summary.label,
            display_name: summary.display_name,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Account", skip_serializing_if_is_none)]
struct AccountOutput {
    account_id: String,
    source_id: String,
    source_name: String,
    provider: String,
    auth_kind: AuthKindOutput,
    label: Option<String>,
    display_name: Option<String>,
    /// The account this one was merged into. A merged account has no events of its own.
    merged_into_account_id: Option<String>,
    /// As the gateway last described the credential.
    account_type: Option<String>,
    plan: Option<String>,
    /// CLIProxy's status for the credential, such as `active` or `error`.
    status: Option<String>,
    /// Turned off in CLIProxy.
    disabled: bool,
    /// An explicit 401 expired-token answer; only a successful quota read clears it.
    sign_in_error: Option<SignInError>,
    /// Absent unless sign-in expired and this provider has a verified management UI action.
    reauthentication: Option<Reauthentication>,
    /// When CLIProxy's credential list first left the credential out. Present while CLIProxy
    /// no longer lists it.
    removed_at: Option<String>,
    /// When the last request charged to the account happened.
    last_used_at: Option<String>,
    first_seen_at: String,
    last_seen_at: String,
    event_count: i64,
    /// Previous sign-ins explicitly merged into this account.
    credentials: Vec<CredentialOutput>,
    /// Advisory replacements of this removed credential, excluding dismissed pairs.
    plan_continuation_account_ids: Vec<String>,
    /// The one credential that replaced this removed one. Present only while no other
    /// credential of the same source, provider, sign-in kind and label could have, the
    /// pair is not dismissed and the replacement has no plan of its own. Derived on every
    /// read, so it appears and goes with `account.changed`. Fold the plan history onto
    /// the successor; the history stays on this account until a merge.
    replaced_by_account_id: Option<String>,
}

impl From<(Account, SignIn)> for AccountOutput {
    fn from((account, sign_in): (Account, SignIn)) -> Self {
        let summary = account.summary;
        Self {
            account_id: summary.id.encode(),
            source_id: summary.source_id.encode(),
            source_name: summary.source_name,
            provider: summary.provider,
            auth_kind: summary.auth_kind.into(),
            label: summary.label,
            display_name: summary.display_name,
            merged_into_account_id: account.merged_into_account_id.map(Id::encode),
            account_type: account.account_type,
            plan: account.plan,
            status: account.status,
            disabled: account.disabled,
            sign_in_error: sign_in.error,
            reauthentication: sign_in.reauthentication,
            removed_at: account.removed_at.map(|at| at.0.to_string()),
            last_used_at: account.last_used_at.map(|at| at.0.to_string()),
            first_seen_at: account.first_seen_at.to_string(),
            last_seen_at: account.last_seen_at.to_string(),
            event_count: account.event_count,
            credentials: account
                .credentials
                .into_iter()
                .map(CredentialOutput::from)
                .collect(),
            plan_continuation_account_ids: account
                .plan_continuation_account_ids
                .into_iter()
                .map(Id::encode)
                .collect(),
            replaced_by_account_id: account.replaced_by_account_id.map(Id::encode),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Credential", skip_serializing_if_is_none)]
struct CredentialOutput {
    account: AccountSummaryOutput,
    first_seen_at: String,
    last_seen_at: String,
}

impl From<Credential> for CredentialOutput {
    fn from(credential: Credential) -> Self {
        Self {
            account: credential.summary.into(),
            first_seen_at: credential.first_seen_at.to_string(),
            last_seen_at: credential.last_seen_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "AuthKind")]
enum AuthKindOutput {
    #[oai(rename = "oauth")]
    OAuth,
    #[oai(rename = "api_key")]
    ApiKey,
    #[oai(rename = "unknown")]
    Unknown,
}

impl From<AuthKind> for AuthKindOutput {
    fn from(kind: AuthKind) -> Self {
        match kind {
            AuthKind::OAuth => Self::OAuth,
            AuthKind::ApiKey => Self::ApiKey,
            AuthKind::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Object)]
struct AccountList {
    accounts: Vec<AccountOutput>,
}

#[derive(Debug, Object)]
struct AccountUpdate {
    display_name: Option<String>,
}

#[derive(Debug, Object)]
struct ContinuationDismissInput {
    other_account_id: String,
}

#[derive(Debug, Object)]
struct MergeInput {
    into_account_id: String,
    /// Rejects a stale suggestion without changing either plan. Absent for manual merges.
    require_plan_continuation: Option<bool>,
}

#[derive(ApiResponse)]
enum DismissResponse {
    #[oai(status = 204)]
    Dismissed,
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum AccountListResponse {
    #[oai(status = 200)]
    Found(Json<AccountList>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum AccountResponse {
    #[oai(status = 200)]
    Found(Json<Box<AccountOutput>>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum MergeResponse {
    #[oai(status = 200)]
    Merged(Json<Box<AccountOutput>>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn missing() -> Error {
    Error {
        message: "account was not found".to_owned(),
    }
}

fn failure(operation: &'static str, error: &DatabaseError) -> Error {
    tracing::error!(%operation, %error, "account operation failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
