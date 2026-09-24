//! Credential health and quota for CLIProxy sources.
//!
//! A soft sync reads CLIProxy's own credential list, which reaches no provider, and runs on
//! a timer. A hard refresh asks each provider through CLIProxy's `api-call`, exactly as the
//! official management UI does, and runs only when asked. CLIProxy keeps nothing of a hard
//! refresh, so the windows stored here are the only record of it.

pub mod calibration;
pub mod credential;
pub mod management;
pub mod prediction;
pub mod provider;
pub mod window;

use std::sync::Arc;
use std::time::Duration;

use futures_util::{StreamExt, stream};
use jiff::Timestamp;
use reqwest::Client;
use tokio::time::MissedTickBehavior;

use crate::account::{Account, AuthKind, NewAccount};
use crate::app::AppState;
use crate::config::SourceConfig;
use crate::database::DatabaseError;
use crate::id::Id;
use crate::quota::credential::QuotaAccount;
use crate::quota::management::{AuthFile, AuthFiles, Management, ManagementError};
use crate::quota::provider::{Provider, RefreshError};
use crate::quota::window::QuotaWindow;
use crate::source::{Source, SourceKind};

pub const SOFT_SYNC_INTERVAL: Duration = Duration::from_secs(180);
/// Credentials of one source refreshed at once, so a refresh of many does not burst the
/// providers behind one gateway.
const HARD_REFRESH_CONCURRENCY: usize = 4;

/// Providers whose credentials are logins even when CLIProxy does not say so.
const OAUTH_PROVIDERS: [&str; 11] = [
    "claude",
    "codex",
    "antigravity",
    "gemini",
    "gemini-cli",
    "qwen",
    "iflow",
    "kimi",
    "xai",
    "meta",
    "devin",
];

/// A source whose credential list could not be read.
#[derive(Debug)]
pub struct SourceFailure {
    pub source: String,
    pub error: ManagementError,
}

#[derive(Debug, Default)]
pub struct Refresh {
    pub refreshed: i64,
    pub failed: i64,
    pub unreachable: Vec<SourceFailure>,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshRequestError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("account was not found")]
    UnknownAccount,
    #[error(
        "the account cannot be hard refreshed: no adapter, or its credential lacks what the adapter needs"
    )]
    NotRefreshable,
    #[error("source {}: {}", .0.source, .0.error)]
    Unreachable(SourceFailure),
}

/// Soft syncs every CLIProxy source now and then every [`SOFT_SYNC_INTERVAL`]. Never hard
/// refreshes.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SOFT_SYNC_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match sync(&state, &state.http, None).await {
                Ok(failures) => {
                    for failure in failures {
                        tracing::warn!(source = %failure.source, error = %failure.error, "soft sync failed");
                    }
                }
                Err(error) => tracing::error!(%error, "soft sync could not be stored"),
            }
        }
    });
}

/// Reads the credential list of every enabled CLIProxy source, or of `only`, and stores
/// it. A source that cannot be read is reported and the others are still synced.
pub async fn sync(
    state: &AppState,
    client: &Client,
    only: Option<Id<Source>>,
) -> Result<Vec<SourceFailure>, DatabaseError> {
    let mut failures = Vec::new();
    for (source, config) in sources(state, only).await? {
        match Management::new(client, config).auth_files().await {
            Ok(listing) => {
                store(state, source.id, &listing).await?;
            }
            Err(error) => failures.push(SourceFailure {
                source: source.name,
                error,
            }),
        }
    }

    Ok(failures)
}

/// Asks the provider of every hard-refreshable credential, or of `only`, for its quota.
/// Each source's credential list is read first, so the adapters see its current entry and
/// the soft state is stored as well. A credential that fails records why on itself.
pub async fn refresh(
    state: &AppState,
    client: &Client,
    only: Option<Id<Account>>,
) -> Result<Refresh, RefreshRequestError> {
    let source_id = match only {
        Some(id) => Some(
            Account::load(&state.database, id)
                .await?
                .ok_or(RefreshRequestError::UnknownAccount)?
                .summary
                .source_id,
        ),
        None => None,
    };
    let mut outcome = Refresh::default();
    let mut found = false;
    for (source, config) in sources(state, source_id).await? {
        let management = Management::new(client, config);
        let listing = match management.auth_files().await {
            Ok(listing) => listing,
            Err(error) => {
                let failure = SourceFailure {
                    source: source.name,
                    error,
                };
                if only.is_some() {
                    return Err(RefreshRequestError::Unreachable(failure));
                }
                outcome.unreachable.push(failure);
                continue;
            }
        };
        let targets = store(state, source.id, &listing)
            .await?
            .into_iter()
            .filter(|(account_id, ..)| only.is_none_or(|only| only == *account_id))
            .filter_map(|(account_id, file, provider)| Some((account_id, file, provider?)))
            .collect::<Vec<_>>();
        found |= !targets.is_empty();
        let management = &management;
        // Built before they are streamed: futures made inside a stream combinator's
        // closure lose their lifetimes and the handler's future stops being `Send`.
        let fetches = targets
            .into_iter()
            .map(|(account_id, file, provider)| async move {
                let auth_index = file.entry.auth_index.as_deref().unwrap_or_default();
                let result = provider.fetch(management, file, auth_index).await;
                (account_id, result, Timestamp::now())
            })
            .collect::<Vec<_>>();
        let results = stream::iter(fetches)
            .buffer_unordered(HARD_REFRESH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for (account_id, result, at) in results {
            record(state, account_id, &result, at).await?;
            match result {
                Ok(_) => outcome.refreshed += 1,
                Err(error) => {
                    tracing::warn!(%account_id, %error, "hard refresh of a credential failed");
                    outcome.failed += 1;
                }
            }
        }
    }
    if only.is_some() && !found {
        return Err(RefreshRequestError::NotRefreshable);
    }

    Ok(outcome)
}

/// The enabled CLIProxy sources that are configured, with their configuration.
async fn sources(
    state: &AppState,
    only: Option<Id<Source>>,
) -> Result<Vec<(Source, &SourceConfig)>, DatabaseError> {
    let mut sources = Vec::new();
    for config in state
        .config
        .sources
        .iter()
        .filter(|config| config.kind == SourceKind::CliProxy)
    {
        let Some(source) = Source::by_key(&state.database, &config.key).await? else {
            continue;
        };
        if source.enabled && only.is_none_or(|only| only == source.id) {
            sources.push((source, config));
        }
    }

    Ok(sources)
}

/// Registers every listed credential and writes what the list says about it, answering
/// each credential's account with the provider that can hard refresh it, if any.
async fn store<'a>(
    state: &AppState,
    source_id: Id<Source>,
    listing: &'a AuthFiles,
) -> Result<Vec<(Id<Account>, &'a AuthFile, Option<Provider>)>, DatabaseError> {
    let mut transaction = state.database.write().await?;
    let mut stored = Vec::with_capacity(listing.files.len());
    for file in &listing.files {
        let entry = &file.entry;
        let Some(auth_index) = entry
            .auth_index
            .as_deref()
            .map(str::trim)
            .filter(|index| !index.is_empty())
        else {
            continue;
        };
        let provider_name = entry.provider().unwrap_or_else(|| "unknown".to_owned());
        let auth_kind = match entry.account_type.as_deref().map(str::trim) {
            Some("oauth") => AuthKind::OAuth,
            Some("api_key") => AuthKind::ApiKey,
            _ if OAUTH_PROVIDERS.contains(&provider_name.as_str()) => AuthKind::OAuth,
            _ => AuthKind::Unknown,
        };
        let label = [&entry.email, &entry.label]
            .into_iter()
            .flatten()
            .map(|label| label.trim())
            .find(|label| !label.is_empty());
        let account = NewAccount::new(auth_index.to_owned(), auth_kind, label);
        let account_id = Account::register(
            &mut transaction,
            state.database.ids.next::<Account>(),
            source_id,
            &provider_name,
            &account,
            listing.observed_at,
        )
        .await?;
        let provider = provider_name
            .parse::<Provider>()
            .ok()
            .filter(|provider| provider.can_refresh(file));
        QuotaAccount::observe(
            &mut transaction,
            account_id,
            file,
            listing.observed_at,
            provider.is_some(),
        )
        .await?;
        stored.push((account_id, file, provider));
    }
    transaction.commit().await?;

    Ok(stored)
}

async fn record(
    state: &AppState,
    account_id: Id<Account>,
    result: &Result<provider::Observation, RefreshError>,
    at: Timestamp,
) -> Result<(), DatabaseError> {
    let mut transaction = state.database.write().await?;
    match result {
        Ok(observation) => {
            QuotaWindow::replace(&mut transaction, account_id, &observation.windows, at).await?;
            QuotaAccount::refreshed(
                &mut transaction,
                account_id,
                at,
                Ok(observation.plan.as_deref()),
            )
            .await?;
        }
        Err(error) => {
            QuotaAccount::refreshed(&mut transaction, account_id, at, Err(&error.to_string()))
                .await?;
        }
    }
    transaction.commit().await?;

    Ok(())
}
