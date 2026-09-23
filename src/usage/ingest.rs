//! How a batch of normalized records from any gateway becomes stored events.

use jiff::Timestamp;

use crate::account::Account;
use crate::app::AppState;
use crate::database::DatabaseError;
use crate::database::codec::StoredTimestamp;
use crate::id::Id;
use crate::source::Source;
use crate::usage::dead_letter::DeadLetter;
use crate::usage::{NewUsageEvent, UsageEvent};

const INSERT: &str = "INSERT INTO usage_event (event_id, source_id, account_id, record_hash, \
     upstream_id, occurred_at, provider, model, model_alias, endpoint, caller, harness, \
     user_agent, session_id, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, \
     cache_write_tokens, unclassified_tokens, total_tokens, token_quality, latency_ms, ttft_ms, \
     streamed, status_code, failed, error_message, service_tier, reasoning_effort, \
     billed_cost_usd) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
     ?, ?, ?) \
     ON CONFLICT (source_id, record_hash) DO NOTHING";

/// One record as a normalizer handed it over.
#[derive(Debug)]
pub enum Incoming {
    Record(Box<NewUsageEvent>),
    /// `payload` is already stripped of every secret the record carried.
    Rejected {
        error: String,
        payload: serde_json::Value,
    },
}

/// What a batch did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngestReport {
    pub accepted: i64,
    pub duplicates: i64,
    pub rejected: i64,
}

impl NewUsageEvent {
    /// Lowercase hex blake3 over every field that tells two requests apart. Each field is
    /// length prefixed, so no two different records can spell the same bytes.
    #[must_use]
    pub fn record_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        let mut text = |value: &str| {
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        };
        text(&self.upstream_id);
        text(&StoredTimestamp::from(self.occurred_at).to_string());
        text(&self.account.key(&self.provider));
        text(&self.provider);
        text(&self.model);
        text(&self.endpoint);
        for count in [
            self.input_tokens,
            self.output_tokens,
            self.reasoning_tokens,
            self.cache_read_tokens,
            self.cache_write_tokens,
            self.unclassified_tokens,
            self.total_tokens,
            self.status_code,
        ] {
            hasher.update(&count.to_le_bytes());
        }
        hasher.update(&[u8::from(self.failed)]);
        for measure in [self.latency_ms, self.ttft_ms] {
            match measure {
                None => hasher.update(&[0]),
                Some(value) => hasher.update(&[1]).update(&value.to_le_bytes()),
            };
        }

        hasher.finalize().to_hex().to_string()
    }
}

/// Stores a batch in one transaction, so a batch lands whole or not at all. A record the
/// source already delivered is a duplicate, and a rejected one is kept as a dead letter.
/// Committed events are then published to live subscribers.
pub async fn ingest(
    state: &AppState,
    source_id: Id<Source>,
    records: Vec<Incoming>,
) -> Result<IngestReport, DatabaseError> {
    let database = &state.database;
    let received_at = Timestamp::now();
    let mut report = IngestReport::default();
    let mut stored = Vec::with_capacity(records.len());
    let mut transaction = database.write().await?;
    for record in records {
        let event = match record {
            Incoming::Record(event) => event,
            Incoming::Rejected { error, payload } => {
                tracing::warn!(%error, "usage record was not readable");
                DeadLetter::insert(
                    &mut transaction,
                    database.ids.next(),
                    source_id,
                    received_at,
                    &error,
                    &payload,
                )
                .await?;
                report.rejected += 1;
                continue;
            }
        };
        let account_id = Account::resolve(
            &mut transaction,
            database.ids.next(),
            source_id,
            &event.provider,
            &event.account,
            event.occurred_at,
        )
        .await?;
        let id = database.ids.next::<UsageEvent>();
        let inserted = sqlx::query(INSERT)
            .bind(id)
            .bind(source_id)
            .bind(account_id)
            .bind(event.record_hash())
            .bind(&event.upstream_id)
            .bind(StoredTimestamp::from(event.occurred_at))
            .bind(&event.provider)
            .bind(&event.model)
            .bind(&event.model_alias)
            .bind(&event.endpoint)
            .bind(&event.caller)
            .bind(&event.harness)
            .bind(&event.user_agent)
            .bind(&event.session_id)
            .bind(event.input_tokens)
            .bind(event.output_tokens)
            .bind(event.reasoning_tokens)
            .bind(event.cache_read_tokens)
            .bind(event.cache_write_tokens)
            .bind(event.unclassified_tokens)
            .bind(event.total_tokens)
            .bind(event.token_quality)
            .bind(event.latency_ms)
            .bind(event.ttft_ms)
            .bind(event.streamed)
            .bind(event.status_code)
            .bind(event.failed)
            .bind(&event.error_message)
            .bind(&event.service_tier)
            .bind(&event.reasoning_effort)
            .bind(event.billed_cost_usd)
            .execute(&mut *transaction)
            .await?;
        if inserted.rows_affected() > 0 {
            stored.push(id);
            report.accepted += 1;
        } else {
            report.duplicates += 1;
        }
    }
    transaction.commit().await?;
    publish(state, &stored).await;

    Ok(report)
}

/// The batch is already committed, so a stream that misses it here finds it in the
/// database on reconnect, and a failure to read it back is logged rather than refused.
async fn publish(state: &AppState, stored: &[Id<UsageEvent>]) {
    if state.usage.receiver_count() == 0 {
        return;
    }
    match UsageEvent::load_all(&state.database, stored).await {
        Ok(events) => {
            for event in events {
                // Every subscriber may have left since the count was read.
                if state.usage.send(event).is_err() {
                    break;
                }
            }
        }
        Err(error) => tracing::error!(%error, "committed usage events were not published"),
    }
}
