//! Single requests, thinned evenly per model, for plotting one point per request.

use jiff::Timestamp;
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::analytics::AnalyticsError;
use crate::analytics::filter::AnalyticsFilter;
use crate::database::codec::StoredTimestamp;
use crate::prelude::*;

/// A failed request or one without a latency says nothing about how fast a model answers.
const TIMED: &str = " AND usage_event.failed = 0 AND usage_event.latency_ms IS NOT NULL";

#[derive(Debug, FromRow)]
pub struct RequestSample {
    #[sqlx(try_from = "StoredTimestamp")]
    pub occurred_at: Timestamp,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub latency_ms: i64,
    pub ttft_ms: Option<i64>,
}

#[derive(Debug)]
pub struct RequestSamples {
    /// Every successful, timed request the filter selects, sampled or not.
    pub requests: i64,
    /// Oldest first.
    pub samples: Vec<RequestSample>,
}

#[derive(FromRow)]
struct Population {
    requests: i64,
    models: i64,
}

impl RequestSamples {
    /// At most `limit` requests, shared evenly between the models so a rarely used model
    /// keeps its points beside a busy one. A model with more requests than its share keeps
    /// every n-th of them in time order.
    pub async fn load(
        database: &Database,
        filter: &AnalyticsFilter,
        limit: i64,
    ) -> Result<Self, AnalyticsError> {
        filter.check(database).await?;
        let mut count = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(*) AS requests, COUNT(DISTINCT usage_event.model) AS models \
             FROM usage_event",
        );
        filter.push_where(&mut count);
        count.push(TIMED);
        let population = count
            .build_query_as::<Population>()
            .fetch_one(&database.pool)
            .await?;
        if population.models == 0 {
            return Ok(Self {
                requests: 0,
                samples: Vec::new(),
            });
        }
        let quota = (limit + population.models - 1) / population.models;

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT occurred_at, model, input_tokens, output_tokens, latency_ms, ttft_ms \
             FROM (SELECT usage_event.occurred_at, usage_event.model, \
             usage_event.input_tokens, usage_event.output_tokens, usage_event.latency_ms, \
             usage_event.ttft_ms, \
             ROW_NUMBER() OVER (PARTITION BY usage_event.model \
                 ORDER BY usage_event.occurred_at, usage_event.event_id) - 1 AS position, \
             COUNT(*) OVER (PARTITION BY usage_event.model) AS model_requests \
             FROM usage_event",
        );
        filter.push_where(&mut builder);
        builder
            .push(TIMED)
            .push(") WHERE position % ((model_requests + ")
            .push_bind(quota)
            .push(" - 1) / ")
            .push_bind(quota)
            .push(") = 0 ORDER BY occurred_at");
        let samples = builder
            .build_query_as::<RequestSample>()
            .fetch_all(&database.pool)
            .await?;

        Ok(Self {
            requests: population.requests,
            samples,
        })
    }
}
