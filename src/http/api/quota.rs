use std::sync::Arc;

use poem::Request;
use poem::http::Method;
use poem_openapi::OpenApi;
use poem_openapi::param::Query;
use poem_openapi::payload::Json;

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::quota_output::{
    QuotaCooldownResetInput, QuotaList, QuotaListResponse, QuotaListing, QuotaRefresh,
    QuotaRefreshInput, QuotaRefreshPermissions, QuotaRefreshResponse, QuotaResetInput,
    QuotaSyncInput, QuotaSyncResponse,
};
use crate::http::conditional::conditional;
use crate::http::quota_action::{
    share_cooldown_reset, share_hard_refresh, share_quota_reset, share_soft_refresh,
};
use crate::prelude::*;
use crate::quota::credential::QuotaAccount;
use crate::quota::manual_reset::ResetRequestError;
use crate::quota::{RefreshRequestError, SourceFailure};

pub struct QuotaApi {
    pub state: Arc<AppState>,
}

impl QuotaApi {
    pub async fn recovery_refresh(&self, account_id: Id<Account>) -> poem::Response {
        use poem::IntoResponse;
        self.refresh_quota_mode(
            Json(QuotaRefreshInput {
                account_id: Some(account_id.encode()),
            }),
            true,
        )
        .await
        .into_response()
    }

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
    /// The last known state and quota of every CLIProxy credential, from the database only,
    /// and which of `/quota/sync` and `/quota/refresh` the presented token may call.
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
    async fn list_quota(
        &self,
        request: &Request,
        source_id: Query<Option<String>>,
    ) -> QuotaListResponse {
        let source_id = match source_id.0.as_deref().map(str::parse::<Id<Source>>) {
            None => None,
            Some(Ok(id)) => Some(id),
            Some(Err(_)) => {
                return QuotaListResponse::Invalid(Json(message("source_id is not an id")));
            }
        };
        match QuotaAccount::list(&self.state.database, source_id).await {
            Ok(accounts) => QuotaListResponse::Found(Json(QuotaListing {
                accounts: accounts.into_iter().map(Into::into).collect(),
                refresh_permissions: QuotaRefreshPermissions {
                    soft: self
                        .state
                        .credentials
                        .admit(request, &Method::POST, "/quota/sync")
                        .is_some(),
                    hard: self
                        .state
                        .credentials
                        .admit(request, &Method::POST, "/quota/refresh")
                        .is_some(),
                },
            })),
            Err(error) => QuotaListResponse::Failed(Json(failure("list_quota", &error))),
        }
    }

    /// Explicit gateway sync for a human UI gesture. It costs gateway traffic, not provider calls.
    /// The server also syncs gateway lists automatically at startup, every 180 seconds and after ingestion.
    /// These automatic gateway syncs are not provider refreshes or human action journal entries.
    /// Concurrent requests and requests within 60 seconds of completion share the saved result.
    /// Responses identify the original caller, request time and action through X-Quota headers.
    /// Sharing includes operation failures and survives restarts. Invalid requests never fill the reuse window.
    /// After 50 seconds of waiting for claimed work, HTTP 202 returns its `action_id`.
    /// Follow quota.action on /events and read /quota/actions for the final result; never retry.
    /// Live work continues until completion even after disconnects. The next claim closes
    /// abandoned records from a previous process. Cleared reset grants are never restored
    /// by recovery; read the provider grant list before attempting another reset.
    /// App labels never bypass sharing. Saved counts and attribution accompany the current accounts list.
    /// Never poll or schedule this route. Never retry automatically except to recover the same human gesture with its original
    /// Idempotency-Key. Never generate a new key for an automatic retry. Use GET /quota and /events.
    /// Soft refresh reaches the gateway only; provider quota windows remain cached.
    ///
    /// Reads CLIProxy's credential list now, which reaches no provider, and answers every
    /// credential. 502 names each source that could not be read; the others are stored.
    #[oai(
        path = "/quota/sync",
        method = "post",
        operation_id = "sync_quota",
        transform = "share_soft_refresh",
        request_header(
            name = "X-Quota-Client",
            ty = "String",
            description = "Required app label, 1 to 80 printable ASCII characters. Self-reported, not proof of human identity. The journal separately records the authenticated credential role and transport."
        ),
        request_header(
            name = "Idempotency-Key",
            ty = "Option<String>",
            description = "Optional key for one human gesture, 1 to 128 graphic ASCII characters. Reuse it for the same operation and scope to recover its result across timeouts and restarts, beyond the 60-second coalescing window. A different operation or scope returns 409. An unknown result never starts another operation for that key; after human reconciliation, a new deliberate gesture uses a new key."
        ),
        response_header(
            name = "X-Quota-Action-Id",
            ty = "Option<String>",
            description = "The durable action whose result this response carries."
        ),
        response_header(
            name = "X-Quota-Requested-By",
            ty = "Option<String>",
            description = "The app label on the original action."
        ),
        response_header(
            name = "X-Quota-Requested-At",
            ty = "Option<String>",
            description = "When the original action was requested, RFC 3339."
        ),
        response_header(
            name = "X-Quota-Finished-At",
            ty = "Option<String>",
            description = "When the shared action completed, RFC 3339. Absent for an unknown outcome."
        ),
        response_header(
            name = "X-Quota-Reused",
            ty = "Option<String>",
            description = "true when this request started no new operation and received a saved result."
        )
    )]
    async fn sync_quota(&self, input: Json<QuotaSyncInput>) -> QuotaSyncResponse {
        self.sync_quota_inner(input).await
    }

    /// Costly operation intended only for direct human invocation, such as a UI button press.
    /// Concurrent requests and requests within 60 seconds of completion share the saved result.
    /// Responses identify the original caller, request time and action through X-Quota headers.
    /// Sharing includes operation failures and survives restarts. Invalid requests never fill the reuse window.
    /// After 50 seconds of waiting for claimed work, HTTP 202 returns its `action_id`.
    /// Follow quota.action on /events and read /quota/actions for the final result; never retry.
    /// Live work continues until completion even after disconnects. The next claim closes
    /// abandoned records from a previous process. Cleared reset grants are never restored
    /// by recovery; read the provider grant list before attempting another reset.
    /// App labels never bypass sharing. Saved counts and attribution accompany the current accounts list.
    /// Never poll or schedule this route. Never retry automatically except to recover the same human gesture with its original
    /// Idempotency-Key. Never generate a new key for an automatic retry. Use GET /quota and /events.
    ///
    /// Automatic sign-in recovery uses only non-spending quota/status readers, waits 3 minutes
    /// from discovering an expired account, then doubles to at most 6 hours. It shares
    /// history/deduplication with human actions. Only successful provider reads clear backoff;
    /// gateway actions, failed reads and human reuse do not reset it. Scheduled reservations
    /// advance backoff even when they reuse a human result without another provider read.
    /// Paid probes, including xAI chat,
    /// always require a human. Meta key creation is also human-only. Gateway sync is unchanged.
    /// An account can reuse a preceding bulk response, including its original scope and counts.
    /// Bulk requests skip recently refreshed accounts and identify them in `shared_actions`.
    /// A reset's follow-up observation is also reused, so refresh does not repeat it.
    ///
    /// Asks each provider for its quota through CLIProxy, as the official management UI
    /// does, and answers every credential. Without `account_id` every hard-refreshable
    /// credential is refreshed. A credential whose provider fails records the reason in
    /// `hard_refresh_error` and counts in `failed`; it does not fail the request.
    /// Bulk source failures appear in `source_failures` alongside successful counts, with HTTP 200.
    #[oai(
        path = "/quota/refresh",
        method = "post",
        operation_id = "refresh_quota",
        transform = "share_hard_refresh",
        request_header(
            name = "X-Quota-Client",
            ty = "String",
            description = "Required app label, 1 to 80 printable ASCII characters. Self-reported, not proof of human identity. The journal separately records the authenticated credential role and transport."
        ),
        request_header(
            name = "Idempotency-Key",
            ty = "Option<String>",
            description = "Optional key for one human gesture, 1 to 128 graphic ASCII characters. Reuse it for the same operation and scope to recover its result across timeouts and restarts, beyond the 60-second coalescing window. A different operation or scope returns 409. An unknown result never starts another operation for that key; after human reconciliation, a new deliberate gesture uses a new key."
        ),
        response_header(
            name = "X-Quota-Action-Id",
            ty = "Option<String>",
            description = "The durable action whose result this response carries."
        ),
        response_header(
            name = "X-Quota-Requested-By",
            ty = "Option<String>",
            description = "The app label on the original action."
        ),
        response_header(
            name = "X-Quota-Requested-At",
            ty = "Option<String>",
            description = "When the original action was requested, RFC 3339."
        ),
        response_header(
            name = "X-Quota-Finished-At",
            ty = "Option<String>",
            description = "When the shared action completed, RFC 3339. Absent for an unknown outcome."
        ),
        response_header(
            name = "X-Quota-Reused",
            ty = "Option<String>",
            description = "true when this request started no new operation and received a saved result."
        )
    )]
    async fn refresh_quota(&self, input: Json<QuotaRefreshInput>) -> QuotaRefreshResponse {
        self.refresh_quota_inner(input).await
    }

    /// Costly, irreversible operation intended only for direct human invocation with confirmation.
    /// Concurrent requests and requests within 60 seconds of completion share the saved result.
    /// Responses identify the original caller, request time and action through X-Quota headers.
    /// Sharing includes operation failures and survives restarts. Invalid requests never fill the reuse window.
    /// After 50 seconds of waiting for claimed work, HTTP 202 returns its `action_id`.
    /// Follow quota.action on /events and read /quota/actions for the final result; never retry.
    /// Live work continues until completion even after disconnects. The next claim closes
    /// abandoned records from a previous process. Cleared reset grants are never restored
    /// by recovery; read the provider grant list before attempting another reset.
    /// App labels never bypass sharing. Saved counts and attribution accompany the current accounts list.
    /// Never poll or schedule this route. Never retry automatically except to recover the same human gesture
    /// with its original Idempotency-Key. Never generate a new key for an automatic retry.
    /// OpenAI/Codex resets are supported. Anthropic grants are currently read-only.
    ///
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
    #[oai(
        path = "/quota/reset",
        method = "post",
        operation_id = "reset_quota",
        transform = "share_quota_reset",
        request_header(
            name = "X-Quota-Client",
            ty = "String",
            description = "Required app label, 1 to 80 printable ASCII characters. Self-reported, not proof of human identity. The journal separately records the authenticated credential role and transport."
        ),
        request_header(
            name = "Idempotency-Key",
            ty = "Option<String>",
            description = "Optional key for one human gesture, 1 to 128 graphic ASCII characters. Reuse it for the same operation and scope to recover its result across timeouts and restarts, beyond the 60-second coalescing window. A different operation or scope returns 409. An unknown result never starts another operation for that key; after human reconciliation, a new deliberate gesture uses a new key."
        ),
        response_header(
            name = "X-Quota-Action-Id",
            ty = "Option<String>",
            description = "The durable action whose result this response carries."
        ),
        response_header(
            name = "X-Quota-Requested-By",
            ty = "Option<String>",
            description = "The app label on the original action."
        ),
        response_header(
            name = "X-Quota-Requested-At",
            ty = "Option<String>",
            description = "When the original action was requested, RFC 3339."
        ),
        response_header(
            name = "X-Quota-Finished-At",
            ty = "Option<String>",
            description = "When the shared action completed, RFC 3339. Absent for an unknown outcome."
        ),
        response_header(
            name = "X-Quota-Reused",
            ty = "Option<String>",
            description = "true when this request started no new operation and received a saved result."
        )
    )]
    async fn reset_quota(&self, input: Json<QuotaResetInput>) -> QuotaRefreshResponse {
        self.reset_quota_inner(input).await
    }

    /// Clears one account's CLIProxy scheduling cooldowns without spending a provider reset or fetching provider quota.
    /// Costly operation intended only for direct human invocation, such as a UI button press.
    /// Concurrent requests and requests within 60 seconds of completion share the saved result.
    /// Responses identify the original caller, request time and action through X-Quota headers.
    /// Sharing includes operation failures and survives restarts. Invalid requests never fill the reuse window.
    /// After 50 seconds of waiting for claimed work, HTTP 202 returns its `action_id`.
    /// Follow quota.action on /events and read /quota/actions for the final result; never retry.
    /// Live work continues until completion even after disconnects. The next claim closes
    /// abandoned records from a previous process. Cleared reset grants are never restored
    /// by recovery; read the provider grant list before attempting another reset.
    /// App labels never bypass sharing. Saved counts and attribution accompany the current accounts list.
    /// Never poll or schedule this operation. Never retry automatically except to recover the same human gesture with its original
    /// Idempotency-Key. Never generate a new key for an automatic retry. Read GET /quota and follow
    /// /events for routine updates. `account_id` is required; there is no reset-all operation.
    /// refreshed counts confirmed cooldown clears, failed counts unconfirmed clears.
    #[oai(
        path = "/quota/cooldowns/reset",
        method = "post",
        operation_id = "reset_quota_cooldowns",
        transform = "share_cooldown_reset",
        request_header(
            name = "X-Quota-Client",
            ty = "String",
            description = "Required app label, 1 to 80 printable ASCII characters. Self-reported, not proof of human identity. The journal separately records the authenticated credential role and transport."
        ),
        request_header(
            name = "Idempotency-Key",
            ty = "Option<String>",
            description = "Optional key for one human gesture, 1 to 128 graphic ASCII characters. Reuse it for the same operation and scope to recover its result across timeouts and restarts, beyond the 60-second coalescing window. A different operation or scope returns 409. An unknown result never starts another operation for that key; after human reconciliation, a new deliberate gesture uses a new key."
        ),
        response_header(
            name = "X-Quota-Action-Id",
            ty = "Option<String>",
            description = "The durable action whose result this response carries."
        ),
        response_header(
            name = "X-Quota-Requested-By",
            ty = "Option<String>",
            description = "The app label on the original action."
        ),
        response_header(
            name = "X-Quota-Requested-At",
            ty = "Option<String>",
            description = "When the original action was requested, RFC 3339."
        ),
        response_header(
            name = "X-Quota-Finished-At",
            ty = "Option<String>",
            description = "When the shared action completed, RFC 3339. Absent for an unknown outcome."
        ),
        response_header(
            name = "X-Quota-Reused",
            ty = "Option<String>",
            description = "true when this request started no new operation and received a saved result."
        )
    )]
    async fn reset_cooldowns(&self, input: Json<QuotaCooldownResetInput>) -> QuotaRefreshResponse {
        self.reset_cooldowns_inner(input).await
    }
}

impl QuotaApi {
    async fn sync_quota_inner(&self, input: Json<QuotaSyncInput>) -> QuotaSyncResponse {
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
    async fn refresh_quota_inner(&self, input: Json<QuotaRefreshInput>) -> QuotaRefreshResponse {
        self.refresh_quota_mode(input, false).await
    }
    async fn refresh_quota_mode(
        &self,
        input: Json<QuotaRefreshInput>,
        automatic: bool,
    ) -> QuotaRefreshResponse {
        let only = match input.0.account_id.as_deref().map(str::parse::<Id<Account>>) {
            None => None,
            Some(Ok(id)) => Some(id),
            Some(Err(_)) => {
                return QuotaRefreshResponse::Missing(Json(message("account was not found")));
            }
        };
        let result = match (automatic, only) {
            (true, Some(id)) => {
                crate::quota::refresh_automatic(&self.state, &self.state.http, id).await
            }
            (true, None) => Err(RefreshRequestError::NotRefreshable),
            (false, only) => crate::quota::refresh(&self.state, &self.state.http, only).await,
        };
        let refresh = match result {
            Ok(refresh) => refresh,
            Err(RefreshRequestError::UnknownAccount) => {
                return QuotaRefreshResponse::Missing(Json(message("account was not found")));
            }
            Err(error @ (RefreshRequestError::NotRefreshable | RefreshRequestError::NotListed)) => {
                return QuotaRefreshResponse::Invalid(Json(message(&error.to_string())));
            }
            Err(error @ RefreshRequestError::Unreachable(_)) => {
                return QuotaRefreshResponse::Unreachable(Json(message(&error.to_string())));
            }
            Err(RefreshRequestError::Database(error)) => {
                return QuotaRefreshResponse::Failed(Json(failure("refresh_quota", &error)));
            }
        };
        match self.everything("refresh_quota").await {
            Ok(list) => QuotaRefreshResponse::Refreshed(Json(QuotaRefresh {
                refreshed: refresh.refreshed,
                failed: refresh.failed,
                source_failures: refresh.unreachable.into_iter().map(Into::into).collect(),
                shared_actions: refresh.shared_actions,
                accounts: list.accounts,
            })),
            Err(error) => QuotaRefreshResponse::Failed(Json(error)),
        }
    }
    async fn reset_quota_inner(&self, input: Json<QuotaResetInput>) -> QuotaRefreshResponse {
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
                source_failures: reset.unreachable.into_iter().map(Into::into).collect(),
                shared_actions: reset.shared_actions,
                accounts: list.accounts,
            })),
            Err(error) => QuotaRefreshResponse::Failed(Json(error)),
        }
    }

    async fn reset_cooldowns_inner(
        &self,
        input: Json<QuotaCooldownResetInput>,
    ) -> QuotaRefreshResponse {
        let Ok(account_id) = input.0.account_id.parse::<Id<Account>>() else {
            return QuotaRefreshResponse::Missing(Json(message("account was not found")));
        };
        let refresh = match crate::quota::cooldown::reset(&self.state, account_id).await {
            Ok(refresh) => refresh,
            Err(RefreshRequestError::UnknownAccount) => {
                return QuotaRefreshResponse::Missing(Json(message("account was not found")));
            }
            Err(error @ (RefreshRequestError::NotRefreshable | RefreshRequestError::NotListed)) => {
                return QuotaRefreshResponse::Invalid(Json(message(&error.to_string())));
            }
            Err(error @ RefreshRequestError::Unreachable(_)) => {
                return QuotaRefreshResponse::Unreachable(Json(message(&error.to_string())));
            }
            Err(RefreshRequestError::Database(error)) => {
                return QuotaRefreshResponse::Failed(Json(failure("reset_cooldowns", &error)));
            }
        };
        match self.everything("reset_cooldowns").await {
            Ok(list) => QuotaRefreshResponse::Refreshed(Json(QuotaRefresh {
                refreshed: refresh.refreshed,
                failed: refresh.failed,
                source_failures: refresh.unreachable.into_iter().map(Into::into).collect(),
                shared_actions: refresh.shared_actions,
                accounts: list.accounts,
            })),
            Err(error) => QuotaRefreshResponse::Failed(Json(error)),
        }
    }
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
