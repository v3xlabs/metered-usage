use jiff::Timestamp;
use reqwest::Client;
use sqlx::{FromRow, SqliteConnection};

use crate::app::AppState;
use crate::change::snapshot::{Scope, Snapshot};
use crate::database::codec::StoredTimestamp;
use crate::prelude::*;
use crate::quota::management::Management;
use crate::quota::provider::RefreshError;
use crate::quota::{Refresh, SourceFailure, record, sources, store};

/// A manual reset grant the provider reported, with at least one reset left. A Codex credit
/// is one reset with no label or total; a Claude grant holds `remaining` of `total` resets
/// under its label. `expires_at` is absent when the provider gives no expiry date.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Grant {
    pub label: Option<String>,
    pub remaining: i64,
    pub total: Option<i64>,
    pub expires_at: Option<StoredTimestamp>,
}

#[derive(Debug, FromRow)]
pub struct Credit {
    pub account_id: Id<Account>,
    #[sqlx(flatten)]
    pub grant: Grant,
}

impl Credit {
    /// Soonest expiry first, grants without one last.
    pub async fn list_unexpired(
        database: &Database,
        source_id: Option<Id<Source>>,
        now: Timestamp,
    ) -> Result<Vec<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(
            "SELECT manual_reset.account_id, manual_reset.label, manual_reset.remaining, \
             manual_reset.total, manual_reset.expires_at FROM manual_reset \
             JOIN account ON account.account_id = manual_reset.account_id \
             WHERE (?1 IS NULL OR account.source_id = ?1) \
             AND (manual_reset.expires_at IS NULL OR manual_reset.expires_at > ?2) \
             ORDER BY manual_reset.account_id, manual_reset.expires_at IS NULL, \
             manual_reset.expires_at",
        )
        .bind(source_id)
        .bind(StoredTimestamp::from(now))
        .fetch_all(&database.pool)
        .await?)
    }

    pub async fn replace(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
        grants: &[Grant],
    ) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM manual_reset WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *connection)
            .await?;
        for grant in grants {
            sqlx::query(
                "INSERT INTO manual_reset (account_id, label, remaining, total, expires_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(account_id)
            .bind(&grant.label)
            .bind(grant.remaining)
            .bind(grant.total)
            .bind(grant.expires_at)
            .execute(&mut *connection)
            .await?;
        }

        Ok(())
    }

    pub async fn unexpired_count(
        database: &Database,
        account_id: Id<Account>,
        now: Timestamp,
    ) -> Result<i64, DatabaseError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(remaining), 0) FROM manual_reset \
             WHERE account_id = ? AND (expires_at IS NULL OR expires_at > ?)",
        )
        .bind(account_id)
        .bind(StoredTimestamp::from(now))
        .fetch_one(&database.pool)
        .await?)
    }

    pub async fn clear(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
    ) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM manual_reset WHERE account_id = ?")
            .bind(account_id)
            .execute(connection)
            .await?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResetRequestError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("account was not found")]
    UnknownAccount,
    #[error("no enabled CLIProxy source lists the credential")]
    NotListed,
    #[error("the credential is disabled in CLIProxy")]
    Disabled,
    #[error(
        "the account cannot be reset: its provider's manual resets cannot be spent here, or its credential lacks what spending one needs"
    )]
    NotResettable,
    #[error("the account has no unexpired manual reset")]
    NoneAvailable,
    #[error("source {}: {}", .0.source, .0.error)]
    Unreachable(SourceFailure),
    #[error("the provider did not confirm the manual reset: {0}")]
    OutcomeUnknown(RefreshError),
}

pub async fn spend(
    state: &AppState,
    client: &Client,
    account_id: Id<Account>,
) -> Result<Refresh, ResetRequestError> {
    let account = Account::load(&state.database, account_id)
        .await?
        .ok_or(ResetRequestError::UnknownAccount)?;
    let reset_guard = state.manual_reset_lock.lock().await;
    let Some((source, config)) = sources(state, Some(account.summary.source_id))
        .await?
        .into_iter()
        .next()
    else {
        return Err(ResetRequestError::NotListed);
    };
    let management = Management::new(client, config);
    let listing = match management.auth_files().await {
        Ok(listing) => listing,
        Err(error) => {
            return Err(ResetRequestError::Unreachable(SourceFailure {
                source: source.name,
                error,
            }));
        }
    };
    let (file, provider) = store(state, source.id, &listing)
        .await?
        .into_iter()
        .find(|(id, ..)| *id == account_id)
        .map(|(_, file, provider)| (file, provider))
        .ok_or(ResetRequestError::NotListed)?;
    if file.entry.disabled {
        return Err(ResetRequestError::Disabled);
    }
    let provider = provider
        .filter(|provider| provider.can_reset(file))
        .ok_or(ResetRequestError::NotResettable)?;
    if Credit::unexpired_count(&state.database, account_id, Timestamp::now()).await? == 0 {
        return Err(ResetRequestError::NoneAvailable);
    }
    let mut transaction = state.database.write().await?;
    let snapshot = Snapshot::quota(&mut transaction, Scope::Account(account_id)).await?;
    Credit::clear(&mut transaction, account_id).await?;
    snapshot.record_changes(&mut transaction).await?;
    transaction.commit().await?;
    let auth_index = file.entry.auth_index.as_deref().unwrap_or_default();
    provider
        .reset(&management, file, auth_index)
        .await
        .map_err(ResetRequestError::OutcomeUnknown)?;

    let result = match management.reset_cooldown(auth_index).await {
        Ok(()) => provider.fetch(&management, file, auth_index).await,
        Err(error) => Err(RefreshError::from(error)),
    };
    record(state, account_id, &result, Timestamp::now()).await?;
    drop(reset_guard);

    Ok(match result {
        Ok(_) => Refresh {
            refreshed: 1,
            ..Refresh::default()
        },
        Err(error) => {
            tracing::warn!(
                %account_id,
                %error,
                "a manual reset was spent but the quota was not refreshed"
            );
            Refresh {
                failed: 1,
                ..Refresh::default()
            }
        }
    })
}
