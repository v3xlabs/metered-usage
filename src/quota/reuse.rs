//! Account observations already obtained by another human action, including a bulk action.

use jiff::{SignedDuration, Timestamp};
use poem_openapi::Object;
use sqlx::FromRow;

use crate::prelude::*;

/// A minute absorbs duplicate human clicks across apps without suppressing a later
/// deliberate refresh indefinitely. It starts at completion, so slow upstream calls do
/// not shorten protection. All HTTP callers and bulk/account scopes share this window.
pub const REUSE_FOR: SignedDuration = SignedDuration::from_secs(60);

#[derive(Debug, Clone, Object, FromRow)]
#[oai(rename = "SharedQuotaAction")]
pub struct SharedQuotaAction {
    pub action_id: i64,
    pub requested_by: String,
    pub requested_at: String,
    pub finished_at: String,
}

#[derive(FromRow)]
pub struct Reuse {
    #[sqlx(flatten)]
    pub action: SharedQuotaAction,
    pub failed: bool,
}

impl Reuse {
    pub async fn recent(
        database: &Database,
        kind: &str,
        account_id: Id<Account>,
        requested_at: Timestamp,
    ) -> Result<Option<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(
            "SELECT q.action_id, q.requested_by, q.requested_at, q.finished_at, \
             CASE WHEN ?1 = 'hard_refresh' THEN json_extract(a.value, '$.hard_refresh_error') IS NOT NULL \
             ELSE (COALESCE(json_extract(q.response_body, '$.failed'), 1) > 0 OR COALESCE(json_array_length(q.response_body, '$.source_failures'), 0) > 0) END AS failed \
             FROM quota_action AS q, json_each(COALESCE(q.after_state, q.response_body), '$.accounts') AS a \
             WHERE q.shared = 1 AND q.response_status IS NOT NULL AND q.finished_at >= ?3 \
             AND (q.kind = ?1 OR (?1 = 'hard_refresh' AND q.kind = 'quota_reset')) \
             AND json_extract(a.value, '$.account.account_id') = ?2 \
             AND (?1 != 'cooldown_reset' OR json_extract(a.value, '$.disabled') = 0) \
             AND (?1 != 'hard_refresh' OR (json_extract(a.value, '$.hard_refreshed_at') >= q.requested_at \
             AND NOT EXISTS (SELECT 1 FROM quota_action AS newer WHERE newer.kind = 'quota_reset' \
             AND newer.account_id = ?2 AND newer.action_id > q.action_id))) \
             ORDER BY q.action_id DESC LIMIT 1",
        ).bind(kind).bind(account_id.encode()).bind((requested_at - REUSE_FOR).to_string())
            .fetch_optional(&database.pool).await?)
    }
}
