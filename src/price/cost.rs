//! The two costs an event carries: its tokens at list rates, and what was billed for it.

use jiff::Timestamp;
use serde_json::Value;
use sqlx::{Connection, SqliteConnection};

use crate::app::AppState;
use crate::change::{Change, Subject};
use crate::config::PricingConfig;
use crate::database::DatabaseError;
use crate::database::codec::StoredTimestamp;
use crate::id::Id;
use crate::price::ModelPrice;
use crate::price::effort::ModelCache;
use crate::usage::{PricedEvent, UsageEvent, UsageUpdate};

/// Picks one price per unpriced event and freezes its cost onto the event. Events no
/// price matches are left untouched, so they are not rewritten on every pass.
///
/// The provider condition applies to manual rows only: gateways and LiteLLM name
/// providers differently (`claude` against `anthropic`), and a map key names one model.
/// LiteLLM keys some models only under their provider (`meta/muse-wonder-1.0`), so such a
/// key also prices the bare name when the gateway names that same provider. `?1` is the
/// config's `priced_as` as a JSON object, which names the key for a model the map knows
/// only under another name (`kimi-k3-256k` as `moonshot/kimi-k3`). A bare key still wins,
/// because the exact name ranks first.
macro_rules! fill_statement {
    ($filter:literal) => {
        concat!(
            "WITH priced_as AS (SELECT key AS model, value AS price_model \
         FROM json_each(?1)), \
     resolved_as AS (SELECT json_extract(value, '$.provider') AS provider, \
         json_extract(value, '$.model') AS model, \
         json_extract(value, '$.price_model') AS price_model \
         FROM json_each(?2)), \
     matched AS ( \
     SELECT event.event_id, price.price_id, \
         price.model = event.model AS exact_model, \
         NOT (price.model = event.model \
             OR COALESCE(price.model = event.model_alias, 0) \
             OR COALESCE(price.model = priced_as.price_model, 0) \
             OR (price.model = event.provider || '/' || event.model \
                 AND price.provider = event.provider)) AS from_resolved, \
         price.service_tier, price.origin, price.min_input_tokens, \
         price.provider, price.effective_from \
     FROM usage_event AS event \
     LEFT JOIN priced_as ON priced_as.model = event.model \
     LEFT JOIN resolved_as \
         ON resolved_as.provider = event.provider AND resolved_as.model = event.model \
     JOIN model_price AS price \
         ON (price.model = event.model OR price.model = event.model_alias \
             OR price.model = priced_as.price_model \
             OR price.model = resolved_as.price_model \
             OR (price.model = event.provider || '/' || event.model \
                 AND price.provider = event.provider)) \
         AND (price.origin <> 'manual' OR price.provider IS NULL \
             OR price.provider = event.provider) \
         AND (price.service_tier IS NULL OR price.service_tier = event.service_tier) \
         AND price.min_input_tokens <= event.input_tokens \
         AND price.effective_from <= event.occurred_at \
         WHERE ",
            $filter,
            "), \
     ranked AS ( \
     SELECT event_id, price_id, from_resolved, ROW_NUMBER() OVER ( \
         PARTITION BY event_id \
         ORDER BY exact_model DESC, \
             from_resolved ASC, \
             service_tier IS NOT NULL DESC, \
             CASE origin WHEN 'manual' THEN 0 WHEN 'litellm_live' THEN 1 ELSE 2 END, \
             min_input_tokens DESC, \
             provider IS NOT NULL DESC, \
             effective_from DESC, \
             price_id DESC) AS preference \
     FROM matched), \
     chosen AS (SELECT event_id, price_id, from_resolved FROM ranked WHERE preference = 1) \
     UPDATE usage_event SET price_id = chosen.price_id, \
     list_cost_usd = (MAX(usage_event.input_tokens - usage_event.cache_read_tokens \
             - usage_event.cache_write_tokens, 0) * price.input_usd_per_mtok \
         + usage_event.cache_read_tokens * price.cached_input_usd_per_mtok \
         + usage_event.cache_write_tokens * price.cache_write_usd_per_mtok \
         + usage_event.output_tokens * price.output_usd_per_mtok) / 1e6, \
     billed_cost_usd = CASE WHEN chosen.from_resolved THEN usage_event.billed_cost_usd \
         ELSE COALESCE(usage_event.billed_cost_usd, \
             CASE WHEN account.auth_kind = 'oauth' THEN 0.0 ELSE \
             (MAX(usage_event.input_tokens - usage_event.cache_read_tokens \
                 - usage_event.cache_write_tokens, 0) * price.input_usd_per_mtok \
             + usage_event.cache_read_tokens * price.cached_input_usd_per_mtok \
             + usage_event.cache_write_tokens * price.cache_write_usd_per_mtok \
             + usage_event.output_tokens * price.output_usd_per_mtok) / 1e6 END) END \
     FROM chosen, model_price AS price, account \
     WHERE usage_event.event_id = chosen.event_id \
     AND price.price_id = chosen.price_id \
     AND account.account_id = usage_event.account_id \
     RETURNING usage_event.event_id, usage_event.price_id, usage_event.list_cost_usd, \
     usage_event.billed_cost_usd"
        )
    };
}

const FILL_ALL: &str = fill_statement!("event.list_cost_usd IS NULL");

/// `?3` is the JSON array of ids; a plain `IN` makes SQLite look each up by primary key.
const FILL_BATCH: &str = fill_statement!(
    "event.event_id IN (SELECT value FROM json_each(?3)) AND event.list_cost_usd IS NULL"
);

impl ModelPrice {
    /// Answers how many events were priced, and tells the live stream what each costs
    /// once that is committed.
    pub async fn fill(state: &AppState) -> Result<usize, DatabaseError> {
        let mut transaction = state.database.write().await?;
        let priced = fill(
            &mut transaction,
            &state.config.pricing,
            &state.database.models,
            None,
        )
        .await?;
        if !priced.is_empty() {
            Change::record(&mut transaction, &Subject::Usage).await?;
        }
        transaction.commit().await?;
        let count = priced.len();
        publish(state, priced);

        Ok(count)
    }

    /// Prices the events a batch just stored, inside its transaction; only a pricing failure is logged.
    pub(crate) async fn price_stored(
        connection: &mut SqliteConnection,
        state: &AppState,
        stored: &[Id<UsageEvent>],
    ) -> Result<(), DatabaseError> {
        if stored.is_empty() {
            return Ok(());
        }
        let mut savepoint = connection.begin().await?;
        match fill(
            &mut savepoint,
            &state.config.pricing,
            &state.database.models,
            Some(stored),
        )
        .await
        {
            Ok(_) => savepoint.commit().await?,
            Err(error) => {
                tracing::warn!(%error, "pricing a stored batch failed, the fill prices it later");
                savepoint.rollback().await?;
            }
        }

        Ok(())
    }

    /// Forgets the cost of every event in the range and prices it again. A billed cost is
    /// kept only where the upstream set it, which only a LiteLLM source does.
    pub async fn reprice(
        state: &AppState,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<u64, DatabaseError> {
        let mut transaction = state.database.write().await?;
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
        let priced = fill(
            &mut transaction,
            &state.config.pricing,
            &state.database.models,
            None,
        )
        .await?;
        if cleared > 0 || !priced.is_empty() {
            Change::record(&mut transaction, &Subject::Usage).await?;
        }
        transaction.commit().await?;
        publish(state, priced);

        Ok(cleared)
    }
}

/// Prices every unpriced event, or only `only` when given.
async fn fill(
    connection: &mut SqliteConnection,
    pricing: &PricingConfig,
    models: &ModelCache,
    only: Option<&[Id<UsageEvent>]>,
) -> Result<Vec<PricedEvent>, DatabaseError> {
    let priced_as = pricing
        .priced_as
        .iter()
        .map(|(model, key)| (model, key.as_str()))
        .collect::<Value>();
    let ids = only.map(|ids| ids.iter().map(|id| id.raw()).collect::<Value>().to_string());
    let unpriced: Vec<(String, String)> = match &ids {
        Some(ids) => {
            sqlx::query_as(
                "SELECT DISTINCT provider, model FROM usage_event \
                 WHERE event_id IN (SELECT value FROM json_each(?1)) AND list_cost_usd IS NULL",
            )
            .bind(ids)
            .fetch_all(&mut *connection)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT DISTINCT provider, model FROM usage_event WHERE list_cost_usd IS NULL",
            )
            .fetch_all(&mut *connection)
            .await?
        }
    };
    let resolved_as = models.resolved_as_json(&unpriced);
    let statement =
        sqlx::query_as::<_, PricedEvent>(if ids.is_some() { FILL_BATCH } else { FILL_ALL })
            .bind(priced_as.to_string())
            .bind(resolved_as);
    Ok(match ids {
        Some(ids) => statement.bind(ids).fetch_all(connection).await?,
        None => statement.fetch_all(connection).await?,
    })
}

/// Only after the commit, so a client never sees a cost the database does not hold.
fn publish(state: &AppState, priced: Vec<PricedEvent>) {
    if state.usage.receiver_count() == 0 {
        return;
    }
    for event in priced {
        // Every subscriber may have left since the count was read.
        if state.usage.send(UsageUpdate::Priced(event)).is_err() {
            break;
        }
    }
}
