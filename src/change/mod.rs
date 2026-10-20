//! Changes to the quota, resets, prices, accounts, plans and usage other services read.
//!
//! Each change commits with its write, in cursor order. Usage records one invalidation
//! per changed batch rather than one per request. It names no client or request.

pub mod snapshot;

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use sqlx::FromRow;
use tokio::time::Instant;

use crate::app::AppState;
use crate::database::Write;
use crate::database::codec::StoredTimestamp;
use crate::plan::Plan;
use crate::prelude::*;

/// How long a change is kept. A reader whose cursor is older has to read the lists again,
/// so this is the longest a consumer can be away and still catch up: a week covers a
/// service that was down over a weekend. Usage invalidations share this retention.
pub const RETENTION: SignedDuration = SignedDuration::from_hours(7 * 24);
/// The most changes one read answers. The public price map read for the first time records
/// one change per model, about 3500, so a reader catches up in a few reads.
pub const PAGE: i64 = 500;
/// A change outlives the retention by at most this much. Pruning is one indexed delete.
const PRUNE_EVERY: Duration = Duration::from_hours(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    QuotaUpdated,
    ResetUpdated,
    PriceChanged,
    AccountChanged,
    PlanChanged,
    UsageUpdated,
}

/// What changed, and so which list to read again.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    /// What the account and quota lists say about the account itself: its names, its
    /// credential's state in CLIProxy, and the outcome of its last hard refresh.
    Account(Id<Account>),
    /// A quota window of the account was first reported, reported with other values, or
    /// no longer reported.
    Window {
        account_id: Id<Account>,
        window_key: String,
    },
    /// The manual resets the provider grants the account.
    Resets(Id<Account>),
    /// The latest price of the model from one of its origins.
    Price { model: String },
    /// A plan was added, edited or removed.
    Plan(Id<Plan>),
    /// Usage counts or stored costs changed. Read the summary and leverage again.
    Usage,
}

#[derive(Debug)]
pub struct Change {
    /// Higher for every later commit, and never handed out twice.
    pub cursor: i64,
    pub recorded_at: Timestamp,
    pub subject: Subject,
}

/// What a reader finds after its cursor.
#[derive(Debug)]
pub enum Page {
    Changes {
        /// Oldest first, at most [`PAGE`].
        changes: Vec<Change>,
        /// The cursor to read on from: the last change answered when the page is full,
        /// otherwise the latest cursor, past every change the kinds left out.
        cursor: i64,
    },
    /// Pruning deleted changes the reader has not seen, or the cursor is ahead of the
    /// latest, as one from another database can be. A cursor from another database that is
    /// not ahead goes unnoticed. The reader reads the lists again and follows on from `latest`.
    Expired {
        /// The lowest cursor still answered.
        oldest: i64,
        latest: i64,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a change kind")]
pub struct UnknownChangeKind(pub String);

#[derive(FromRow)]
struct StoredChange {
    cursor: i64,
    kind: String,
    account_id: Option<Id<Account>>,
    window_key: Option<String>,
    model: Option<String>,
    plan_id: Option<Id<Plan>>,
    recorded_at: StoredTimestamp,
}

impl ChangeKind {
    pub const ALL: [Self; 6] = [
        Self::QuotaUpdated,
        Self::ResetUpdated,
        Self::PriceChanged,
        Self::AccountChanged,
        Self::PlanChanged,
        Self::UsageUpdated,
    ];

    #[must_use]
    pub fn stored(self) -> &'static str {
        match self {
            Self::QuotaUpdated => "quota.updated",
            Self::ResetUpdated => "reset.updated",
            Self::PriceChanged => "price.changed",
            Self::AccountChanged => "account.changed",
            Self::PlanChanged => "plan.changed",
            Self::UsageUpdated => "usage.updated",
        }
    }
}

impl FromStr for ChangeKind {
    type Err = UnknownChangeKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.stored() == text)
            .ok_or_else(|| UnknownChangeKind(text.to_owned()))
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stored())
    }
}

impl Subject {
    #[must_use]
    pub fn kind(&self) -> ChangeKind {
        match self {
            Self::Account(_) => ChangeKind::AccountChanged,
            Self::Window { .. } => ChangeKind::QuotaUpdated,
            Self::Resets(_) => ChangeKind::ResetUpdated,
            Self::Price { .. } => ChangeKind::PriceChanged,
            Self::Plan(_) => ChangeKind::PlanChanged,
            Self::Usage => ChangeKind::UsageUpdated,
        }
    }
}

impl Change {
    /// Records the change in the write's transaction, so it commits with what changed.
    pub async fn record(write: &mut Write<'_>, subject: &Subject) -> Result<(), DatabaseError> {
        let (account_id, window_key, model, plan_id) = match subject {
            Subject::Account(account_id) | Subject::Resets(account_id) => {
                (Some(*account_id), None, None, None)
            }
            Subject::Window {
                account_id,
                window_key,
            } => (Some(*account_id), Some(window_key.as_str()), None, None),
            Subject::Price { model } => (None, None, Some(model.as_str()), None),
            Subject::Plan(plan_id) => (None, None, None, Some(*plan_id)),
            Subject::Usage => (None, None, None, None),
        };
        sqlx::query(
            "INSERT INTO change (kind, account_id, window_key, model, plan_id, recorded_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(subject.kind().stored())
        .bind(account_id)
        .bind(window_key)
        .bind(model)
        .bind(plan_id)
        .bind(StoredTimestamp::from(Timestamp::now()))
        .execute(&mut **write)
        .await?;
        write.mark_changed();

        Ok(())
    }

    /// The cursor of the last change committed, or of the last one pruned when none is
    /// kept. 0 before the first change.
    pub async fn latest(database: &Database) -> Result<i64, DatabaseError> {
        Ok(sqlx::query_scalar(
            "SELECT MAX(COALESCE((SELECT MAX(cursor) FROM change), 0), pruned_through) \
             FROM change_horizon",
        )
        .fetch_one(&database.pool)
        .await?)
    }

    /// The changes after `cursor` of the given kinds, or of every kind when none is given.
    pub async fn after(
        database: &Database,
        cursor: i64,
        kinds: &[ChangeKind],
    ) -> Result<Page, DatabaseError> {
        // One read transaction, so the horizon, the changes and the latest cursor all
        // describe the same commit.
        let mut read = database.pool.begin().await?;
        let (oldest, latest) = sqlx::query_as::<_, (i64, i64)>(
            "SELECT pruned_through, \
             MAX(COALESCE((SELECT MAX(cursor) FROM change), 0), pruned_through) \
             FROM change_horizon",
        )
        .fetch_one(&mut *read)
        .await?;
        if cursor < oldest || cursor > latest {
            return Ok(Page::Expired { oldest, latest });
        }
        let kinds = (!kinds.is_empty()).then(|| {
            serde_json::Value::from(kinds.iter().map(|kind| kind.stored()).collect::<Vec<_>>())
                .to_string()
        });
        let changes = sqlx::query_as::<_, StoredChange>(
            "SELECT cursor, kind, account_id, window_key, model, plan_id, recorded_at FROM change \
             WHERE cursor > ?1 AND (?2 IS NULL OR kind IN (SELECT value FROM json_each(?2))) \
             ORDER BY cursor LIMIT ?3",
        )
        .bind(cursor)
        .bind(kinds)
        .bind(PAGE)
        .fetch_all(&mut *read)
        .await?
        .into_iter()
        .map(Change::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        read.commit().await?;
        let cursor = match changes.last() {
            Some(last) if i64::try_from(changes.len()) == Ok(PAGE) => last.cursor,
            _ => latest,
        };

        Ok(Page::Changes { changes, cursor })
    }

    /// [`Self::after`], but while it finds no change and `until` has not passed, waits for
    /// the next commit that records one instead of answering an empty page.
    pub async fn wait(
        database: &Database,
        mut cursor: i64,
        kinds: &[ChangeKind],
        until: Instant,
    ) -> Result<Page, DatabaseError> {
        let mut wake = database.subscribe_to_changes();
        loop {
            let page = Self::after(database, cursor, kinds).await?;
            match &page {
                Page::Changes {
                    changes,
                    cursor: next,
                } if changes.is_empty() => cursor = *next,
                _ => return Ok(page),
            }
            tokio::select! {
                woken = wake.changed() => if woken.is_err() {
                    return Ok(page);
                },
                () = tokio::time::sleep_until(until) => return Ok(page),
            }
        }
    }

    /// Deletes every change recorded before `now` less [`RETENTION`] and raises the
    /// horizon over them. Answers how many went.
    pub async fn prune(database: &Database, now: Timestamp) -> Result<u64, DatabaseError> {
        let before = StoredTimestamp::from(now - RETENTION);
        let mut write = database.write().await?;
        sqlx::query(
            "UPDATE change_horizon SET pruned_through = MAX(pruned_through, \
             COALESCE((SELECT MAX(cursor) FROM change WHERE recorded_at < ?), 0))",
        )
        .bind(before)
        .execute(&mut *write)
        .await?;
        let pruned = sqlx::query("DELETE FROM change WHERE recorded_at < ?")
            .bind(before)
            .execute(&mut *write)
            .await?;
        write.commit().await?;

        Ok(pruned.rows_affected())
    }
}

impl TryFrom<StoredChange> for Change {
    type Error = DatabaseError;

    fn try_from(stored: StoredChange) -> Result<Self, Self::Error> {
        let unreadable = || DatabaseError::Unreadable {
            field: "change",
            value: format!(
                "{} {} {:?} {:?} {:?} {:?}",
                stored.cursor,
                stored.kind,
                stored.account_id,
                stored.window_key,
                stored.model,
                stored.plan_id
            ),
        };
        let kind = stored
            .kind
            .parse::<ChangeKind>()
            .map_err(|_| unreadable())?;
        let subject = match (
            kind,
            stored.account_id,
            &stored.window_key,
            &stored.model,
            stored.plan_id,
        ) {
            (ChangeKind::AccountChanged, Some(account_id), None, None, None) => {
                Subject::Account(account_id)
            }
            (ChangeKind::QuotaUpdated, Some(account_id), Some(window_key), None, None) => {
                Subject::Window {
                    account_id,
                    window_key: window_key.clone(),
                }
            }
            (ChangeKind::ResetUpdated, Some(account_id), None, None, None) => {
                Subject::Resets(account_id)
            }
            (ChangeKind::PriceChanged, None, None, Some(model), None) => Subject::Price {
                model: model.clone(),
            },
            (ChangeKind::PlanChanged, None, None, None, Some(plan_id)) => Subject::Plan(plan_id),
            (ChangeKind::UsageUpdated, None, None, None, None) => Subject::Usage,
            _ => return Err(unreadable()),
        };

        Ok(Self {
            cursor: stored.cursor,
            recorded_at: stored.recorded_at.0,
            subject,
        })
    }
}

/// Prunes once now and then every hour, for as long as the process runs.
pub fn spawn_pruning(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PRUNE_EVERY);
        loop {
            interval.tick().await;
            match Change::prune(&state.database, Timestamp::now()).await {
                Ok(0) => {}
                Ok(pruned) => tracing::info!(pruned, "expired changes were deleted"),
                Err(error) => tracing::error!(%error, "changes were not pruned"),
            }
        }
    });
}
