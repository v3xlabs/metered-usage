use std::collections::BTreeMap;

use jiff::Timestamp;
use sqlx::{AssertSqlSafe, FromRow, SqliteConnection};

use crate::account::continuation::Continuation;
use crate::account::{Account, AccountSummary};
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;

/// `event_count` counts the events charged to the row itself, so a merged account, whose
/// events moved to the account that absorbed it, reads zero.
const SELECT: &str = "SELECT account.account_id, account.source_id, source.name AS source_name, \
     account.provider, account.auth_kind, account.label, account.display_name, \
     account.merged_into_account_id, credential_state.account_type, credential_state.plan, \
     credential_state.status, COALESCE(credential_state.disabled, 0) AS disabled, \
     credential_state.removed_at, account.first_seen_at, account.last_seen_at, \
     (SELECT MAX(occurred_at) FROM usage_event \
         WHERE usage_event.account_id = account.account_id) AS last_used_at, \
     (SELECT COUNT(*) FROM usage_event WHERE usage_event.account_id = account.account_id) AS event_count \
     FROM account JOIN source ON source.source_id = account.source_id \
     LEFT JOIN credential_state ON credential_state.account_id = account.account_id";

/// A previous sign-in. Its seen window stays on its original row after an explicit merge.
#[derive(Debug, FromRow)]
pub struct Credential {
    #[sqlx(flatten)]
    pub summary: AccountSummary,
    #[sqlx(try_from = "StoredTimestamp")]
    pub first_seen_at: Timestamp,
    #[sqlx(try_from = "StoredTimestamp")]
    pub last_seen_at: Timestamp,
}

#[derive(FromRow)]
struct MergedCredential {
    target: Id<Account>,
    #[sqlx(flatten)]
    credential: Credential,
}

impl Account {
    /// Leaves out every account merged into another.
    pub async fn list(database: &Database) -> Result<Vec<Self>, DatabaseError> {
        let mut transaction = database.pool.begin().await?;
        let mut accounts = sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} WHERE account.merged_into_account_id IS NULL \
             ORDER BY account.last_seen_at DESC, account.account_id"
        )))
        .fetch_all(&mut *transaction)
        .await?;
        Self::read_details(&mut transaction, &mut accounts, None).await?;
        transaction.commit().await?;
        Ok(accounts)
    }

    pub async fn load(database: &Database, id: Id<Account>) -> Result<Option<Self>, DatabaseError> {
        let mut transaction = database.pool.begin().await?;
        let mut account = sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} WHERE account.account_id = ?"
        )))
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(account) = account.as_mut() {
            Self::read_details(&mut transaction, std::slice::from_mut(account), Some(id)).await?;
        }
        transaction.commit().await?;
        Ok(account)
    }

    async fn read_details(
        connection: &mut SqliteConnection,
        accounts: &mut [Self],
        account_id: Option<Id<Self>>,
    ) -> Result<(), DatabaseError> {
        let mut by_id: BTreeMap<_, _> = accounts
            .iter_mut()
            .map(|account| (account.summary.id, account))
            .collect();
        let credentials = sqlx::query_as::<_, MergedCredential>(
            "SELECT account.account_id, account.source_id, source.name AS source_name, \
             account.provider, account.auth_kind, account.label, account.display_name, \
             account.merged_into_account_id AS target, account.first_seen_at, account.last_seen_at \
             FROM account JOIN source ON source.source_id = account.source_id \
             WHERE account.merged_into_account_id IS NOT NULL \
             AND (?1 IS NULL OR account.merged_into_account_id = ?1) ORDER BY account.account_id",
        )
        .bind(account_id)
        .fetch_all(&mut *connection)
        .await?;
        for credential in credentials {
            if let Some(account) = by_id.get_mut(&credential.target) {
                account.credentials.push(credential.credential);
            }
        }
        for suggestion in Continuation::list(connection, account_id).await? {
            if let Some(account) = by_id.get_mut(&suggestion.previous) {
                account
                    .plan_continuation_account_ids
                    .push(suggestion.replacement);
                if suggestion.is_replacement {
                    account.replaced_by_account_id = Some(suggestion.replacement);
                }
            }
        }
        Ok(())
    }
}
