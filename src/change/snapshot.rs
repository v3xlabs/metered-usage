//! What a reader sees of each account, quota window and manual reset grant, read in a
//! write's transaction before and after it writes, so the write records exactly the changes
//! it made and none it did not, whichever of its statements made them.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use sqlx::SqliteConnection;

use crate::change::{Change, Subject};
use crate::database::Write;
use crate::database::codec::StoredTimestamp;
use crate::plan::Plan;
use crate::prelude::*;

/// Everything the account and quota lists show of an account other than its traffic. A
/// cooldown's `remaining_seconds` counts down on every read of CLIProxy's list, so only
/// what the cooldown is, and when it ends, is compared.
const ACCOUNTS: &str = "SELECT account.account_id, json_array(source.name, account.provider, \
     account.auth_kind, account.label, account.display_name, account.merged_into_account_id, \
     credential_state.status, credential_state.status_message, credential_state.disabled, \
     credential_state.unavailable, credential_state.next_retry_after, \
     (SELECT json_group_array(json_array(json_extract(cooldown.value, '$.scope'), \
         json_extract(cooldown.value, '$.model_key'), json_extract(cooldown.value, '$.reason'), \
         json_extract(cooldown.value, '$.retry_at'))) \
         FROM json_each(credential_state.cooldowns) AS cooldown), \
     credential_state.account_type, credential_state.plan, credential_state.hard_refreshable, \
     credential_state.hard_refreshed_at, credential_state.hard_refresh_error, \
     credential_state.removed_at, \
     (SELECT json_group_array(json_array(previous.account_id, previous.label, previous.display_name, \
         previous.provider, previous.auth_kind, previous.first_seen_at, previous.last_seen_at) \
         ORDER BY previous.account_id) FROM account AS previous \
         WHERE previous.merged_into_account_id = account.account_id)) \
     FROM account JOIN source ON source.source_id = account.source_id \
     LEFT JOIN credential_state ON credential_state.account_id = account.account_id \
     WHERE (?1 IS NULL OR account.source_id = ?1) AND (?2 IS NULL OR account.account_id = ?2 \
         OR account.account_id = (SELECT merged_into_account_id FROM account WHERE account_id = ?2))";

/// The account each plan is on and the credential that replaced it, so a plan moved by a
/// merge or reached by a new replacement is recorded as the plan lists show it.
const PLANS: &str = "SELECT plan.plan_id, plan.account_id FROM plan \
     JOIN account ON account.account_id = plan.account_id \
     WHERE (?1 IS NULL OR account.source_id = ?1) AND (?2 IS NULL OR account.account_id = ?2 \
         OR account.account_id = (SELECT merged_into_account_id FROM account WHERE account_id = ?2))";

/// A window's values as the provider reported them. When it was reported is left out: a
/// hard refresh that read the same values changed nothing a reader acts on, and its time
/// already changes the account.
const WINDOWS: &str = "SELECT quota_window.account_id, quota_window.window_key, \
     json_array(quota_window.label, quota_window.used_fraction, quota_window.used_value, \
     quota_window.limit_value, quota_window.unit, quota_window.window_seconds, \
     quota_window.resets_at, quota_window.scope, quota_window.starts_on_use) \
     FROM quota_window JOIN account ON account.account_id = quota_window.account_id \
     WHERE (?1 IS NULL OR account.source_id = ?1) AND (?2 IS NULL OR account.account_id = ?2)";

/// Every unexpired grant of an account, as the quota list shows them. A grant that expires
/// on its own records nothing; the list says when it expires.
const RESETS: &str = "SELECT manual_reset.account_id, \
     json_group_array(json_array(manual_reset.label, manual_reset.remaining, \
         manual_reset.total, manual_reset.expires_at) \
         ORDER BY manual_reset.expires_at, manual_reset.label, manual_reset.remaining, \
         manual_reset.total) \
     FROM manual_reset JOIN account ON account.account_id = manual_reset.account_id \
     WHERE (?1 IS NULL OR account.source_id = ?1) AND (?2 IS NULL OR account.account_id = ?2) \
     AND (manual_reset.expires_at IS NULL OR manual_reset.expires_at > ?3) \
     GROUP BY manual_reset.account_id";

/// The accounts a write may touch.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    All,
    Source(Id<Source>),
    Account(Id<Account>),
}

/// What of each account a write may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parts {
    Accounts,
    /// The accounts, their quota windows and their manual resets.
    Quota,
}

#[derive(Debug)]
pub struct Snapshot {
    scope: Scope,
    parts: Parts,
    /// The instant both reads count a grant as expired by, so one that expires during the
    /// write is not a change.
    now: Timestamp,
    seen: BTreeMap<Subject, String>,
}

impl Scope {
    fn source(self) -> Option<Id<Source>> {
        match self {
            Self::Source(source_id) => Some(source_id),
            Self::All | Self::Account(_) => None,
        }
    }

    fn account(self) -> Option<Id<Account>> {
        match self {
            Self::Account(account_id) => Some(account_id),
            Self::All | Self::Source(_) => None,
        }
    }
}

impl Snapshot {
    /// What the account lists show of each account in scope.
    pub async fn accounts(
        connection: &mut SqliteConnection,
        scope: Scope,
    ) -> Result<Self, DatabaseError> {
        Self::read(connection, scope, Parts::Accounts, Timestamp::now()).await
    }

    /// What the account and quota lists show of each account in scope, its quota windows
    /// and its manual resets.
    pub async fn quota(
        connection: &mut SqliteConnection,
        scope: Scope,
    ) -> Result<Self, DatabaseError> {
        Self::read(connection, scope, Parts::Quota, Timestamp::now()).await
    }

    /// Reads the same scope again and records a change for every subject that reads
    /// differently now, appeared or went.
    pub async fn record_changes(self, write: &mut Write<'_>) -> Result<(), DatabaseError> {
        let after = Self::read(write, self.scope, self.parts, self.now).await?;
        let changed = self
            .seen
            .iter()
            .filter(|(subject, seen)| after.seen.get(*subject) != Some(*seen))
            .chain(
                after
                    .seen
                    .iter()
                    .filter(|(subject, seen)| self.seen.get(*subject) != Some(*seen)),
            )
            .map(|(subject, _)| subject)
            .collect::<BTreeSet<_>>();
        for subject in changed {
            Change::record(write, subject).await?;
        }

        Ok(())
    }

    async fn read(
        connection: &mut SqliteConnection,
        scope: Scope,
        parts: Parts,
        now: Timestamp,
    ) -> Result<Self, DatabaseError> {
        // A write to one account can move a link of another, since a replacement's plan type
        // is part of what links it to the credential it replaced. So an account's scope reads
        // its whole source, which no write of one account changes beyond that account's own
        // subjects and the links.
        let (source, account) = match scope {
            Scope::Account(account_id) => {
                match sqlx::query_scalar::<_, Id<Source>>(
                    "SELECT source_id FROM account WHERE account_id = ?",
                )
                .bind(account_id)
                .fetch_optional(&mut *connection)
                .await?
                {
                    Some(source_id) => (Some(source_id), None),
                    None => (None, Some(account_id)),
                }
            }
            Scope::All | Scope::Source(_) => (scope.source(), scope.account()),
        };
        let mut seen = BTreeMap::new();
        let replacements = Account::replacements(&mut *connection).await?;
        for (account_id, state) in sqlx::query_as::<_, (Id<Account>, String)>(ACCOUNTS)
            .bind(source)
            .bind(account)
            .fetch_all(&mut *connection)
            .await?
        {
            let replaced_by = replacements.get(&account_id).map(|id| id.encode());
            seen.insert(
                Subject::Account(account_id),
                format!("{state}{}", replaced_by.unwrap_or_default()),
            );
        }
        for (plan_id, account_id) in sqlx::query_as::<_, (Id<Plan>, Id<Account>)>(PLANS)
            .bind(source)
            .bind(account)
            .fetch_all(&mut *connection)
            .await?
        {
            let replaced_by = replacements.get(&account_id).map(|id| id.encode());
            seen.insert(
                Subject::Plan(plan_id),
                format!(
                    "{}>{}",
                    account_id.encode(),
                    replaced_by.unwrap_or_default()
                ),
            );
        }
        if parts == Parts::Quota {
            for (account_id, window_key, window) in
                sqlx::query_as::<_, (Id<Account>, String, String)>(WINDOWS)
                    .bind(source)
                    .bind(account)
                    .fetch_all(&mut *connection)
                    .await?
            {
                seen.insert(
                    Subject::Window {
                        account_id,
                        window_key,
                    },
                    window,
                );
            }
            for (account_id, resets) in sqlx::query_as::<_, (Id<Account>, String)>(RESETS)
                .bind(source)
                .bind(account)
                .bind(StoredTimestamp::from(now))
                .fetch_all(&mut *connection)
                .await?
            {
                seen.insert(Subject::Resets(account_id), resets);
            }
        }

        Ok(Self {
            scope,
            parts,
            now,
            seen,
        })
    }
}
