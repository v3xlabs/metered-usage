//! What CLIProxy last said about a credential, and what its provider last said through it.

use std::collections::HashMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqliteConnection};

use crate::account::{Account, AccountSummary};
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::quota::management::AuthFile;
use crate::quota::window::QuotaWindow;
use crate::source::Source;

const SELECT: &str = "SELECT account.account_id, account.source_id, source.name AS source_name, \
     account.provider, account.auth_kind, account.label, account.display_name, \
     credential_state.status, credential_state.status_message, credential_state.disabled, \
     credential_state.unavailable, credential_state.next_retry_after, credential_state.cooldowns, \
     credential_state.account_type, credential_state.plan, credential_state.soft_observed_at, \
     credential_state.hard_refreshed_at, credential_state.hard_refresh_error, \
     credential_state.hard_refreshable \
     FROM credential_state \
     JOIN account ON account.account_id = credential_state.account_id \
     JOIN source ON source.source_id = account.source_id \
     WHERE account.merged_into_account_id IS NULL AND (?1 IS NULL OR account.source_id = ?1) \
     ORDER BY source.name, account.provider, account.label, account.account_id";

/// A credential CLIProxy has listed, with every quota window its provider last reported.
#[derive(Debug, FromRow)]
pub struct QuotaAccount {
    #[sqlx(flatten)]
    pub account: AccountSummary,
    pub status: Option<String>,
    pub status_message: Option<String>,
    pub disabled: bool,
    pub unavailable: bool,
    /// When CLIProxy will next route to the credential. A scheduling decision of the
    /// gateway, never the provider's quota reset.
    pub next_retry_after: Option<StoredTimestamp>,
    #[sqlx(try_from = "String")]
    pub cooldowns: Cooldowns,
    pub account_type: Option<String>,
    pub plan: Option<String>,
    pub soft_observed_at: Option<StoredTimestamp>,
    pub hard_refreshed_at: Option<StoredTimestamp>,
    pub hard_refresh_error: Option<String>,
    pub hard_refreshable: bool,
    #[sqlx(skip)]
    pub windows: Vec<QuotaWindow>,
}

impl QuotaAccount {
    /// Reads the database only; nothing here reaches CLIProxy.
    pub async fn list(
        database: &Database,
        source_id: Option<Id<Source>>,
    ) -> Result<Vec<Self>, DatabaseError> {
        let mut accounts = sqlx::query_as::<_, Self>(SELECT)
            .bind(source_id)
            .fetch_all(&database.pool)
            .await?;
        let mut windows = QuotaWindow::list(database, source_id)
            .await?
            .into_iter()
            .fold(
                HashMap::<Id<Account>, Vec<QuotaWindow>>::new(),
                |mut grouped, window| {
                    grouped.entry(window.account_id).or_default().push(window);
                    grouped
                },
            );
        for account in &mut accounts {
            account.windows = windows.remove(&account.account.id).unwrap_or_default();
        }

        Ok(accounts)
    }

    /// Writes what the credential list said. The plan is kept when the list names none,
    /// because a hard refresh may have learned it.
    pub async fn observe(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
        file: &AuthFile,
        observed_at: Timestamp,
        hard_refreshable: bool,
    ) -> Result<(), DatabaseError> {
        let entry = &file.entry;
        let cooldowns = serde_json::to_string(entry.cooldowns.as_deref().unwrap_or_default())
            .map_err(|error| DatabaseError::Unreadable {
                field: "cooldowns",
                value: error.to_string(),
            })?;
        let plan = entry
            .id_token
            .as_ref()
            .and_then(|claims| claims.get("plan_type"))
            .and_then(|plan| plan.as_str())
            .map(str::trim)
            .filter(|plan| !plan.is_empty())
            .map(str::to_lowercase);
        sqlx::query(
            "INSERT INTO credential_state (account_id, status, status_message, disabled, \
             unavailable, next_retry_after, cooldowns, account_type, plan, soft_observed_at, \
             hard_refreshable) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (account_id) DO UPDATE SET \
             status = excluded.status, \
             status_message = excluded.status_message, \
             disabled = excluded.disabled, \
             unavailable = excluded.unavailable, \
             next_retry_after = excluded.next_retry_after, \
             cooldowns = excluded.cooldowns, \
             account_type = excluded.account_type, \
             plan = COALESCE(excluded.plan, credential_state.plan), \
             soft_observed_at = excluded.soft_observed_at, \
             hard_refreshable = excluded.hard_refreshable",
        )
        .bind(account_id)
        .bind(entry.status.as_deref().filter(|status| !status.is_empty()))
        .bind(
            entry
                .status_message
                .as_deref()
                .filter(|message| !message.is_empty()),
        )
        .bind(entry.disabled)
        .bind(entry.unavailable)
        .bind(entry.next_retry_after.map(StoredTimestamp::from))
        .bind(cooldowns)
        .bind(
            entry
                .account_type
                .as_deref()
                .filter(|kind| !kind.is_empty()),
        )
        .bind(plan)
        .bind(StoredTimestamp::from(observed_at))
        .bind(hard_refreshable)
        .execute(connection)
        .await?;

        Ok(())
    }

    /// Records one hard refresh attempt. A failure keeps the windows last observed.
    pub async fn refreshed(
        connection: &mut SqliteConnection,
        account_id: Id<Account>,
        at: Timestamp,
        outcome: Result<Option<&str>, &str>,
    ) -> Result<(), DatabaseError> {
        let (plan, error) = match outcome {
            Ok(plan) => (plan, None),
            Err(error) => (None, Some(error)),
        };
        sqlx::query(
            "UPDATE credential_state SET hard_refreshed_at = ?, hard_refresh_error = ?, \
             plan = COALESCE(?, plan) WHERE account_id = ?",
        )
        .bind(StoredTimestamp::from(at))
        .bind(error)
        .bind(plan)
        .bind(account_id)
        .execute(connection)
        .await?;

        Ok(())
    }
}

/// One of CLIProxy's cooldown timers on a credential or on one model of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cooldown {
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_key: Option<String>,
    pub reason: String,
    pub retry_at: Timestamp,
    pub remaining_seconds: i64,
}

#[derive(Debug, Default)]
pub struct Cooldowns(pub Vec<Cooldown>);

impl TryFrom<String> for Cooldowns {
    type Error = serde_json::Error;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        serde_json::from_str(&text).map(Self)
    }
}
