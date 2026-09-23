//! The two costs an event carries: its tokens at list rates, and what was billed for it.

use jiff::Timestamp;
use sqlx::SqliteExecutor;

use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::price::ModelPrice;

/// Picks one price per unpriced event and freezes its cost onto the event. Events no
/// price matches are left untouched, so they are not rewritten on every pass.
///
/// The provider condition applies to manual rows only: gateways and LiteLLM name
/// providers differently (`claude` against `anthropic`), and a map key names one model.
const FILL: &str = "WITH ranked AS ( \
     SELECT event.event_id, price.price_id, ROW_NUMBER() OVER ( \
         PARTITION BY event.event_id \
         ORDER BY price.model = event.model DESC, \
             price.service_tier IS NOT NULL DESC, \
             CASE price.origin WHEN 'manual' THEN 0 WHEN 'litellm_live' THEN 1 ELSE 2 END, \
             price.min_input_tokens DESC, \
             price.provider IS NOT NULL DESC, \
             price.effective_from DESC, \
             price.price_id DESC) AS preference \
     FROM usage_event AS event \
     JOIN model_price AS price \
         ON (price.model = event.model OR price.model = event.model_alias) \
         AND (price.origin <> 'manual' OR price.provider IS NULL \
             OR price.provider = event.provider) \
         AND (price.service_tier IS NULL OR price.service_tier = event.service_tier) \
         AND price.min_input_tokens <= event.input_tokens \
         AND price.effective_from <= event.occurred_at \
     WHERE event.list_cost_usd IS NULL), \
     chosen AS (SELECT event_id, price_id FROM ranked WHERE preference = 1) \
     UPDATE usage_event SET price_id = chosen.price_id, \
     list_cost_usd = (MAX(usage_event.input_tokens - usage_event.cache_read_tokens \
             - usage_event.cache_write_tokens, 0) * price.input_usd_per_mtok \
         + usage_event.cache_read_tokens * price.cached_input_usd_per_mtok \
         + usage_event.cache_write_tokens * price.cache_write_usd_per_mtok \
         + usage_event.output_tokens * price.output_usd_per_mtok) / 1e6, \
     billed_cost_usd = COALESCE(usage_event.billed_cost_usd, \
         CASE WHEN account.auth_kind = 'oauth' THEN 0.0 ELSE \
         (MAX(usage_event.input_tokens - usage_event.cache_read_tokens \
             - usage_event.cache_write_tokens, 0) * price.input_usd_per_mtok \
         + usage_event.cache_read_tokens * price.cached_input_usd_per_mtok \
         + usage_event.cache_write_tokens * price.cache_write_usd_per_mtok \
         + usage_event.output_tokens * price.output_usd_per_mtok) / 1e6 END) \
     FROM chosen, model_price AS price, account \
     WHERE usage_event.event_id = chosen.event_id \
     AND price.price_id = chosen.price_id \
     AND account.account_id = usage_event.account_id";

impl ModelPrice {
    /// Answers how many events were priced.
    pub async fn fill(database: &Database) -> Result<u64, DatabaseError> {
        fill(&database.pool).await
    }

    /// Forgets the cost of every event in the range and prices it again. A billed cost is
    /// kept only where the upstream set it, which only a LiteLLM source does.
    pub async fn reprice(
        database: &Database,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<u64, DatabaseError> {
        let mut transaction = database.write().await?;
        let cleared = sqlx::query(
            "UPDATE usage_event SET price_id = NULL, list_cost_usd = NULL, \
             billed_cost_usd = CASE WHEN (SELECT source.kind FROM source \
                 WHERE source.source_id = usage_event.source_id) = 'litellm' \
                 THEN billed_cost_usd END \
             WHERE (?1 IS NULL OR occurred_at >= ?1) AND (?2 IS NULL OR occurred_at <= ?2)",
        )
        .bind(from.map(StoredTimestamp::from))
        .bind(to.map(StoredTimestamp::from))
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        fill(&mut *transaction).await?;
        transaction.commit().await?;

        Ok(cleared)
    }
}

async fn fill(executor: impl SqliteExecutor<'_>) -> Result<u64, DatabaseError> {
    Ok(sqlx::query(FILL).execute(executor).await?.rows_affected())
}
