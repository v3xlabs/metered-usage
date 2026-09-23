//! The upstream credential whose quota a request consumed.
//!
//! An account is not a person. Two subscriptions logged in with one email are two
//! accounts, and one email seen through two sources is two accounts.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteTypeInfo, SqliteValueRef};
use sqlx::{
    AssertSqlSafe, Database as SqlxDatabase, Decode, Encode, FromRow, Sqlite, SqliteConnection,
    Type,
};

use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::source::Source;

/// `event_count` counts the events charged to the row itself, so a merged account, whose
/// events moved to the account that absorbed it, reads zero.
const SELECT: &str = "SELECT account.account_id, account.source_id, source.name AS source_name, \
     account.provider, account.auth_kind, account.label, account.display_name, \
     account.merged_into_account_id, credential_state.account_type, credential_state.plan, \
     account.first_seen_at, account.last_seen_at, \
     (SELECT COUNT(*) FROM usage_event WHERE usage_event.account_id = account.account_id) AS event_count \
     FROM account JOIN source ON source.source_id = account.source_id \
     LEFT JOIN credential_state ON credential_state.account_id = account.account_id";

/// Shorter than this, the last four characters of a secret are most of it.
const MASKABLE_LENGTH: usize = 8;
const MASK_TAIL: usize = 4;

#[derive(Debug, FromRow)]
pub struct Account {
    #[sqlx(flatten)]
    pub summary: AccountSummary,
    pub merged_into_account_id: Option<Id<Account>>,
    pub account_type: Option<String>,
    pub plan: Option<String>,
    #[sqlx(try_from = "StoredTimestamp")]
    pub first_seen_at: Timestamp,
    #[sqlx(try_from = "StoredTimestamp")]
    pub last_seen_at: Timestamp,
    pub event_count: i64,
}

impl Account {
    /// Leaves out every account merged into another.
    pub async fn list(database: &Database) -> Result<Vec<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} WHERE account.merged_into_account_id IS NULL \
             ORDER BY account.last_seen_at DESC, account.account_id"
        )))
        .fetch_all(&database.pool)
        .await?)
    }

    pub async fn load(database: &Database, id: Id<Account>) -> Result<Option<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} WHERE account.account_id = ?"
        )))
        .bind(id)
        .fetch_optional(&database.pool)
        .await?)
    }

    /// Answers the account as it now reads, or nothing when there is no such account.
    pub async fn rename(
        database: &Database,
        id: Id<Account>,
        display_name: Option<&str>,
    ) -> Result<Option<Self>, DatabaseError> {
        let renamed = sqlx::query("UPDATE account SET display_name = ? WHERE account_id = ?")
            .bind(display_name)
            .bind(id)
            .execute(&database.pool)
            .await?;
        if renamed.rows_affected() == 0 {
            return Ok(None);
        }

        Self::load(database, id).await
    }

    /// Finds the account a record was charged to, creating it the first time the source
    /// reports it, and widens the window it was seen in to cover `seen_at`. A merged
    /// account answers the account that absorbed it, whose window widens instead.
    pub async fn resolve(
        connection: &mut SqliteConnection,
        candidate: Id<Account>,
        source_id: Id<Source>,
        provider: &str,
        account: &NewAccount,
        seen_at: Timestamp,
    ) -> Result<Id<Account>, DatabaseError> {
        let seen_at = StoredTimestamp::from(seen_at);
        // Stored instants are fixed width, so text order is time order and MIN and MAX
        // keep a replayed old record from moving either bound the wrong way.
        let (id, merged_into): (Id<Account>, Option<Id<Account>>) = sqlx::query_as(
            "INSERT INTO account (account_id, source_id, provider, upstream_key, auth_kind, \
             label, first_seen_at, last_seen_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (source_id, upstream_key) DO UPDATE SET \
             provider = excluded.provider, \
             auth_kind = excluded.auth_kind, \
             label = COALESCE(excluded.label, account.label), \
             first_seen_at = MIN(account.first_seen_at, excluded.first_seen_at), \
             last_seen_at = MAX(account.last_seen_at, excluded.last_seen_at) \
             RETURNING account_id, merged_into_account_id",
        )
        .bind(candidate)
        .bind(source_id)
        .bind(provider)
        .bind(account.key(provider).as_ref())
        .bind(account.auth_kind)
        .bind(&account.label)
        .bind(seen_at)
        .bind(seen_at)
        .fetch_one(&mut *connection)
        .await?;
        let Some(target) = merged_into else {
            return Ok(id);
        };
        sqlx::query(
            "UPDATE account SET first_seen_at = MIN(first_seen_at, ?), \
             last_seen_at = MAX(last_seen_at, ?) WHERE account_id = ?",
        )
        .bind(seen_at)
        .bind(seen_at)
        .bind(target)
        .execute(connection)
        .await?;

        Ok(target)
    }

    /// Records a credential the gateway lists, whether or not it has carried traffic. The
    /// seen window is traffic's alone, so an account already known keeps it, and a label
    /// the gateway no longer reports is kept rather than cleared.
    pub async fn register(
        connection: &mut SqliteConnection,
        candidate: Id<Account>,
        source_id: Id<Source>,
        provider: &str,
        account: &NewAccount,
        observed_at: Timestamp,
    ) -> Result<Id<Account>, DatabaseError> {
        let observed_at = StoredTimestamp::from(observed_at);
        Ok(sqlx::query_scalar(
            "INSERT INTO account (account_id, source_id, provider, upstream_key, auth_kind, \
             label, first_seen_at, last_seen_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (source_id, upstream_key) DO UPDATE SET \
             provider = excluded.provider, \
             auth_kind = excluded.auth_kind, \
             label = COALESCE(excluded.label, account.label) \
             RETURNING COALESCE(merged_into_account_id, account_id)",
        )
        .bind(candidate)
        .bind(source_id)
        .bind(provider)
        .bind(account.key(provider).as_ref())
        .bind(account.auth_kind)
        .bind(&account.label)
        .bind(observed_at)
        .bind(observed_at)
        .fetch_one(connection)
        .await?)
    }

    /// Moves every event of `id` onto `into` and points `id`, and every account already
    /// merged into it, at `into`, so resolution never follows more than one step.
    pub async fn merge(
        database: &Database,
        id: Id<Account>,
        into: Id<Account>,
    ) -> Result<Self, MergeError> {
        if id == into {
            return Err(MergeError::Same);
        }
        let mut transaction = database.write().await?;
        let read = "SELECT source_id, merged_into_account_id, first_seen_at, last_seen_at \
                    FROM account WHERE account_id = ?";
        let merged = sqlx::query_as::<_, MergeSide>(read)
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(DatabaseError::from)?
            .ok_or(MergeError::Missing)?;
        let target = sqlx::query_as::<_, MergeSide>(read)
            .bind(into)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(DatabaseError::from)?
            .ok_or(MergeError::Missing)?;
        if merged.source_id != target.source_id {
            return Err(MergeError::DifferentSources);
        }
        if target.merged_into_account_id.is_some() {
            return Err(MergeError::TargetMerged);
        }
        sqlx::query("UPDATE usage_event SET account_id = ? WHERE account_id = ?")
            .bind(into)
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(DatabaseError::from)?;
        sqlx::query(
            "UPDATE account SET merged_into_account_id = ? \
             WHERE account_id = ? OR merged_into_account_id = ?",
        )
        .bind(into)
        .bind(id)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(DatabaseError::from)?;
        sqlx::query(
            "UPDATE account SET first_seen_at = MIN(first_seen_at, ?), \
             last_seen_at = MAX(last_seen_at, ?) WHERE account_id = ?",
        )
        .bind(merged.first_seen_at)
        .bind(merged.last_seen_at)
        .bind(into)
        .execute(&mut *transaction)
        .await
        .map_err(DatabaseError::from)?;
        transaction.commit().await.map_err(DatabaseError::from)?;

        Self::load(database, into).await?.ok_or(MergeError::Missing)
    }
}

/// Why two accounts could not be merged.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("an account cannot be merged into itself")]
    Same,
    #[error("accounts of two sources cannot be merged")]
    DifferentSources,
    #[error("the target account is itself merged into another")]
    TargetMerged,
    #[error("account was not found")]
    Missing,
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

#[derive(FromRow)]
struct MergeSide {
    source_id: Id<Source>,
    merged_into_account_id: Option<Id<Account>>,
    first_seen_at: String,
    last_seen_at: String,
}

/// What a usage event says about the account it was charged to.
#[derive(Debug, Clone, FromRow)]
pub struct AccountSummary {
    #[sqlx(rename = "account_id")]
    pub id: Id<Account>,
    pub source_id: Id<Source>,
    pub source_name: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub label: Option<String>,
    pub display_name: Option<String>,
}

/// An account as a gateway record describes it, with any secret already masked.
#[derive(Debug)]
pub struct NewAccount {
    /// Empty when the record named no credential.
    pub upstream_key: String,
    pub auth_kind: AuthKind,
    pub label: Option<String>,
}

impl NewAccount {
    /// `source` is whatever the gateway said the credential was. Only an OAuth login is an
    /// identity that may be shown; any other kind of source can be the credential itself,
    /// so all that is kept of it is enough of its end to tell two apart.
    #[must_use]
    pub fn new(upstream_key: String, auth_kind: AuthKind, source: Option<&str>) -> Self {
        let source = source.filter(|source| !source.is_empty());
        let label = match auth_kind {
            AuthKind::OAuth => source.map(str::to_owned),
            AuthKind::ApiKey | AuthKind::Unknown => source.and_then(mask),
        };

        Self {
            upstream_key,
            auth_kind,
            label,
        }
    }

    /// The key the account is stored under. Traffic that names no credential shares one
    /// account per provider.
    #[must_use]
    pub fn key(&self, provider: &str) -> Cow<'_, str> {
        if self.upstream_key.is_empty() {
            Cow::Owned(format!("unattributed/{provider}"))
        } else {
            Cow::Borrowed(&self.upstream_key)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    OAuth,
    ApiKey,
    Unknown,
}

impl AuthKind {
    #[must_use]
    pub fn stored(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::ApiKey => "api_key",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not an auth kind")]
pub struct UnknownAuthKind(pub String);

impl FromStr for AuthKind {
    type Err = UnknownAuthKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "oauth" => Ok(Self::OAuth),
            "api_key" => Ok(Self::ApiKey),
            "unknown" => Ok(Self::Unknown),
            other => Err(UnknownAuthKind(other.to_owned())),
        }
    }
}

impl fmt::Display for AuthKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stored())
    }
}

impl Type<Sqlite> for AuthKind {
    fn type_info() -> SqliteTypeInfo {
        <str as Type<Sqlite>>::type_info()
    }

    fn compatible(info: &SqliteTypeInfo) -> bool {
        <str as Type<Sqlite>>::compatible(info)
    }
}

impl<'r> Decode<'r, Sqlite> for AuthKind {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(<&str as Decode<'r, Sqlite>>::decode(value)?.parse()?)
    }
}

impl<'q> Encode<'q, Sqlite> for AuthKind {
    fn encode_by_ref(
        &self,
        buffer: &mut <Sqlite as SqlxDatabase>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <&str as Encode<'q, Sqlite>>::encode(self.stored(), buffer)
    }
}

/// All that may be kept of a secret: enough of its end to tell two apart, or nothing when
/// that end would be most of it.
#[must_use]
pub fn mask(secret: &str) -> Option<String> {
    if secret.chars().count() < MASKABLE_LENGTH {
        return None;
    }
    secret
        .char_indices()
        .rev()
        .nth(MASK_TAIL - 1)
        .map(|(start, _)| format!("…{}", &secret[start..]))
}
