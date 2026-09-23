//! A gateway whose accounting records this service stores. Sources come from the config
//! file alone; the database keeps a row per configured key so events outlive a restart.

pub mod collector_state;

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteRow, SqliteTypeInfo, SqliteValueRef};
use sqlx::{
    AssertSqlSafe, Database as SqlxDatabase, Decode, Encode, FromRow, QueryBuilder, Row, Sqlite,
    Type,
};

use crate::config::Config;
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;
use crate::source::collector_state::CollectorState;

/// The event counts are an aggregate of `usage_event`, not columns of `source`, so every
/// read of a source pays for them once here rather than in each caller.
const SELECT: &str = "SELECT source.source_id, source.key, source.name, source.kind, \
     source.base_url, source.enabled, source.created_at, \
     (SELECT MAX(occurred_at) FROM usage_event WHERE usage_event.source_id = source.source_id) AS last_event_at, \
     (SELECT COUNT(*) FROM usage_event WHERE usage_event.source_id = source.source_id) AS event_count, \
     collector_state.connected_since, collector_state.last_record_at, \
     collector_state.records_received, collector_state.consecutive_failures, \
     collector_state.last_error, collector_state.updated_at AS collector_updated_at \
     FROM source LEFT JOIN collector_state ON collector_state.source_id = source.source_id";

#[derive(Debug)]
pub struct Source {
    pub id: Id<Source>,
    /// The config file's name for the source.
    pub key: String,
    pub name: String,
    pub kind: SourceKind,
    pub base_url: String,
    /// False once the config no longer names it.
    pub enabled: bool,
    pub created_at: Timestamp,
    pub last_event_at: Option<StoredTimestamp>,
    pub event_count: i64,
    /// Absent until a collector first ran for the source.
    pub collector: Option<CollectorState>,
}

impl Source {
    pub async fn load(database: &Database, id: Id<Source>) -> Result<Option<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} WHERE source.source_id = ?"
        )))
        .bind(id)
        .fetch_optional(&database.pool)
        .await?)
    }

    pub async fn by_key(database: &Database, key: &str) -> Result<Option<Self>, DatabaseError> {
        Ok(
            sqlx::query_as::<_, Self>(AssertSqlSafe(format!("{SELECT} WHERE source.key = ?")))
                .bind(key)
                .fetch_optional(&database.pool)
                .await?,
        )
    }

    pub async fn list(database: &Database) -> Result<Vec<Self>, DatabaseError> {
        Ok(
            sqlx::query_as::<_, Self>(AssertSqlSafe(format!("{SELECT} ORDER BY source.source_id")))
                .fetch_all(&database.pool)
                .await?,
        )
    }

    /// Makes the stored sources match the config: every configured one exists and is
    /// enabled with the config's name, kind and URL, and every other one is disabled.
    pub async fn sync(database: &Database, config: &Config) -> Result<(), DatabaseError> {
        let mut transaction = database.write().await?;
        let created_at = StoredTimestamp::from(Timestamp::now());
        for source in &config.sources {
            sqlx::query(
                "INSERT INTO source (source_id, key, name, kind, base_url, enabled, created_at) \
                 VALUES (?, ?, ?, ?, ?, 1, ?) \
                 ON CONFLICT (key) DO UPDATE SET name = excluded.name, kind = excluded.kind, \
                 base_url = excluded.base_url, enabled = 1",
            )
            .bind(database.ids.next::<Self>())
            .bind(&source.key)
            .bind(&source.name)
            .bind(source.kind)
            .bind(&source.base_url)
            .bind(created_at)
            .execute(&mut *transaction)
            .await?;
        }
        let mut disable =
            QueryBuilder::<Sqlite>::new("UPDATE source SET enabled = 0 WHERE enabled = 1");
        if !config.sources.is_empty() {
            disable.push(" AND key NOT IN (");
            let mut keys = disable.separated(", ");
            for source in &config.sources {
                keys.push_bind(source.key.as_str());
            }
            disable.push(")");
        }
        disable.build().execute(&mut *transaction).await?;
        transaction.commit().await?;

        Ok(())
    }
}

impl<'r> FromRow<'r, SqliteRow> for Source {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        let collector = match row.try_get::<Option<StoredTimestamp>, _>("collector_updated_at")? {
            None => None,
            Some(updated_at) => Some(CollectorState {
                connected_since: row
                    .try_get::<Option<StoredTimestamp>, _>("connected_since")?
                    .map(Timestamp::from),
                last_record_at: row
                    .try_get::<Option<StoredTimestamp>, _>("last_record_at")?
                    .map(Timestamp::from),
                records_received: row.try_get("records_received")?,
                consecutive_failures: row.try_get("consecutive_failures")?,
                last_error: row.try_get("last_error")?,
                updated_at: updated_at.into(),
            }),
        };

        Ok(Self {
            id: row.try_get("source_id")?,
            key: row.try_get("key")?,
            name: row.try_get("name")?,
            kind: row.try_get("kind")?,
            base_url: row.try_get("base_url")?,
            enabled: row.try_get("enabled")?,
            created_at: row.try_get::<StoredTimestamp, _>("created_at")?.into(),
            last_event_at: row.try_get("last_event_at")?,
            event_count: row.try_get("event_count")?,
            collector,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    CliProxy,
    LiteLlm,
}

impl SourceKind {
    #[must_use]
    pub fn stored(self) -> &'static str {
        match self {
            Self::CliProxy => "cliproxy",
            Self::LiteLlm => "litellm",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a source kind")]
pub struct UnknownSourceKind(pub String);

impl FromStr for SourceKind {
    type Err = UnknownSourceKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "cliproxy" => Ok(Self::CliProxy),
            "litellm" => Ok(Self::LiteLlm),
            other => Err(UnknownSourceKind(other.to_owned())),
        }
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stored())
    }
}

impl Type<Sqlite> for SourceKind {
    fn type_info() -> SqliteTypeInfo {
        <str as Type<Sqlite>>::type_info()
    }

    fn compatible(info: &SqliteTypeInfo) -> bool {
        <str as Type<Sqlite>>::compatible(info)
    }
}

impl<'r> Decode<'r, Sqlite> for SourceKind {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(<&str as Decode<'r, Sqlite>>::decode(value)?.parse()?)
    }
}

impl<'q> Encode<'q, Sqlite> for SourceKind {
    fn encode_by_ref(
        &self,
        buffer: &mut <Sqlite as SqlxDatabase>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <&str as Encode<'q, Sqlite>>::encode(self.stored(), buffer)
    }
}
