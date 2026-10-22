//! Suggestions only. A credential's provider, email and reported plan type are not its
//! identity. Only an explicit account merge joins history, including an ended plan whose
//! end date must stay ended. A dismissal belongs to the pair, not to the email.
//! A replacement must be newer than the previous sign-in and have no tracked plan.
//! Confirmation rechecks the pair under the existing merge's write lock.

use sqlx::{FromRow, SqliteConnection};

use crate::account::Account;
use crate::change::{Change, Subject};
use crate::database::{Database, DatabaseError};
use crate::id::Id;

const SELECT: &str = "SELECT old.account_id AS previous, new.account_id AS replacement \
    FROM account AS old \
    JOIN credential_state AS old_state ON old_state.account_id = old.account_id \
    JOIN account AS new ON new.source_id = old.source_id AND new.provider = old.provider \
        AND LOWER(TRIM(new.label)) = LOWER(TRIM(old.label)) \
    JOIN credential_state AS new_state ON new_state.account_id = new.account_id \
    WHERE old.merged_into_account_id IS NULL AND new.merged_into_account_id IS NULL \
        AND old.auth_kind = 'oauth' AND new.auth_kind = 'oauth' \
        AND INSTR(old.label, '@') > 1 \
        AND old_state.removed_at IS NOT NULL AND new_state.removed_at IS NULL \
        AND new_state.disabled = 0 AND new_state.soft_observed_at IS NOT NULL \
        AND new.first_seen_at > old.first_seen_at \
        AND LENGTH(TRIM(old_state.plan)) > 0 \
        AND LOWER(TRIM(old_state.plan)) = LOWER(TRIM(new_state.plan)) \
        AND EXISTS (SELECT 1 FROM plan WHERE plan.account_id = old.account_id) \
        AND NOT EXISTS (SELECT 1 FROM plan WHERE plan.account_id = new.account_id) \
        AND NOT EXISTS (SELECT 1 FROM plan_continuation_dismissal \
            WHERE account_low = MIN(old.account_id, new.account_id) \
            AND account_high = MAX(old.account_id, new.account_id)) \
        AND (?1 IS NULL OR old.account_id = ?1) \
        AND (?2 IS NULL OR new.account_id = ?2) \
    ORDER BY old.account_id, new.account_id";

#[derive(FromRow)]
pub(super) struct Continuation {
    pub previous: Id<Account>,
    pub replacement: Id<Account>,
}

impl Continuation {
    pub async fn list(
        connection: &mut SqliteConnection,
        previous: Option<Id<Account>>,
    ) -> Result<Vec<Self>, DatabaseError> {
        Ok(sqlx::query_as(SELECT)
            .bind(previous)
            .bind(None::<Id<Account>>)
            .fetch_all(connection)
            .await?)
    }

    pub async fn load(
        connection: &mut SqliteConnection,
        previous: Id<Account>,
        replacement: Id<Account>,
    ) -> Result<Option<Self>, DatabaseError> {
        Ok(sqlx::query_as(SELECT)
            .bind(previous)
            .bind(replacement)
            .fetch_optional(connection)
            .await?)
    }
}

impl Account {
    /// Idempotent for a pair. The decision changes both accounts' suggestions, not a plan.
    pub async fn dismiss_continuation(
        database: &Database,
        previous: Id<Self>,
        replacement: Id<Self>,
    ) -> Result<bool, DatabaseError> {
        let mut write = database.write().await?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM account AS old JOIN account AS new \
             ON old.source_id = new.source_id WHERE old.account_id = ? \
             AND new.account_id = ? AND old.account_id != new.account_id \
             AND old.merged_into_account_id IS NULL AND new.merged_into_account_id IS NULL)",
        )
        .bind(previous)
        .bind(replacement)
        .fetch_one(&mut *write)
        .await?;
        if !valid {
            return Ok(false);
        }
        let inserted = sqlx::query(
            "INSERT INTO plan_continuation_dismissal (account_low, account_high) \
             VALUES (MIN(?1, ?2), MAX(?1, ?2)) ON CONFLICT DO NOTHING",
        )
        .bind(previous)
        .bind(replacement)
        .execute(&mut *write)
        .await?;
        if inserted.rows_affected() > 0 {
            Change::record(&mut write, &Subject::Account(previous)).await?;
            Change::record(&mut write, &Subject::Account(replacement)).await?;
        }
        write.commit().await?;
        Ok(true)
    }
}
