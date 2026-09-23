use std::sync::Arc;

use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};

use crate::account::MergeError;
use crate::app::AppState;
use crate::http::api::Error;
use crate::prelude::*;

pub struct AccountApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl AccountApi {
    #[oai(path = "/accounts", method = "get", operation_id = "list_accounts")]
    async fn list_accounts(&self) -> AccountListResponse {
        match Account::list(&self.state.database).await {
            Ok(accounts) => AccountListResponse::Found(Json(AccountList {
                accounts: accounts.into_iter().map(AccountOutput::from).collect(),
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
            Ok(Some(account)) => AccountResponse::Found(Json(Box::new(account.into()))),
            Ok(None) => AccountResponse::Missing(Json(missing())),
            Err(error) => AccountResponse::Failed(Json(failure("update_account", &error))),
        }
    }

    /// Folds an account into another of the same source: its events move to the target,
    /// and any record that later names its credential is charged to the target.
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
        match Account::merge(&self.state.database, account_id, into).await {
            Ok(account) => MergeResponse::Merged(Json(Box::new(account.into()))),
            Err(MergeError::Missing) => MergeResponse::Missing(Json(missing())),
            Err(
                error
                @ (MergeError::Same | MergeError::DifferentSources | MergeError::TargetMerged),
            ) => MergeResponse::Invalid(Json(Error {
                message: error.to_string(),
            })),
            Err(MergeError::Database(error)) => {
                MergeResponse::Failed(Json(failure("merge_account", &error)))
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
    first_seen_at: String,
    last_seen_at: String,
    event_count: i64,
}

impl From<Account> for AccountOutput {
    fn from(account: Account) -> Self {
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
            first_seen_at: account.first_seen_at.to_string(),
            last_seen_at: account.last_seen_at.to_string(),
            event_count: account.event_count,
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
struct MergeInput {
    into_account_id: String,
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
