//! The measures every analytics answer reports, whatever it groups by.

use std::cmp::Ordering;
use std::ops::AddAssign;

use sqlx::FromRow;

/// Every column of [`UsageMetrics`], over `usage_event` left joined to the `model_price`
/// row that priced it. Averages are kept as a total and a sample count so that two groups
/// merge exactly. Cache savings read the event's own price row, so an unpriced event adds
/// nothing, and a cache write can make them negative.
pub const METRICS: &str = "COUNT(*) AS requests, \
     SUM(usage_event.failed) AS failures, \
     SUM(usage_event.input_tokens) AS input_tokens, \
     SUM(MAX(usage_event.input_tokens - usage_event.cache_read_tokens \
         - usage_event.cache_write_tokens, 0)) AS uncached_input_tokens, \
     SUM(usage_event.cache_read_tokens) AS cache_read_tokens, \
     SUM(usage_event.cache_write_tokens) AS cache_write_tokens, \
     SUM(usage_event.output_tokens) AS output_tokens, \
     SUM(usage_event.reasoning_tokens) AS reasoning_tokens, \
     SUM(usage_event.unclassified_tokens) AS unclassified_tokens, \
     SUM(usage_event.total_tokens) AS total_tokens, \
     TOTAL(usage_event.list_cost_usd) AS list_cost_usd, \
     TOTAL(usage_event.billed_cost_usd) AS billed_cost_usd, \
     TOTAL((usage_event.cache_read_tokens \
             * (model_price.input_usd_per_mtok - model_price.cached_input_usd_per_mtok) \
         - usage_event.cache_write_tokens \
             * (model_price.cache_write_usd_per_mtok - model_price.input_usd_per_mtok)) \
         / 1e6) AS cache_savings_usd, \
     COUNT(*) - COUNT(usage_event.list_cost_usd) AS unpriced_requests, \
     TOTAL(usage_event.latency_ms) AS latency_ms_total, \
     TOTAL(usage_event.latency_ms IS NOT NULL) AS latency_samples, \
     TOTAL(usage_event.ttft_ms) AS ttft_ms_total, \
     TOTAL(usage_event.ttft_ms IS NOT NULL) AS ttft_samples";

pub const FROM: &str =
    " FROM usage_event LEFT JOIN model_price ON model_price.price_id = usage_event.price_id";

#[derive(Debug, Clone, Copy, Default, PartialEq, FromRow)]
pub struct UsageMetrics {
    pub requests: i64,
    pub failures: i64,
    pub input_tokens: i64,
    pub uncached_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub unclassified_tokens: i64,
    pub total_tokens: i64,
    pub list_cost_usd: f64,
    pub billed_cost_usd: f64,
    pub cache_savings_usd: f64,
    pub unpriced_requests: i64,
    latency_ms_total: f64,
    latency_samples: f64,
    ttft_ms_total: f64,
    ttft_samples: f64,
}

impl UsageMetrics {
    #[must_use]
    pub fn avg_latency_ms(&self) -> Option<f64> {
        (self.latency_samples > 0.0).then(|| self.latency_ms_total / self.latency_samples)
    }

    #[must_use]
    pub fn avg_ttft_ms(&self) -> Option<f64> {
        (self.ttft_samples > 0.0).then(|| self.ttft_ms_total / self.ttft_samples)
    }
}

impl AddAssign<&Self> for UsageMetrics {
    fn add_assign(&mut self, other: &Self) {
        self.requests += other.requests;
        self.failures += other.failures;
        self.input_tokens += other.input_tokens;
        self.uncached_input_tokens += other.uncached_input_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.unclassified_tokens += other.unclassified_tokens;
        self.total_tokens += other.total_tokens;
        self.list_cost_usd += other.list_cost_usd;
        self.billed_cost_usd += other.billed_cost_usd;
        self.cache_savings_usd += other.cache_savings_usd;
        self.unpriced_requests += other.unpriced_requests;
        self.latency_ms_total += other.latency_ms_total;
        self.latency_samples += other.latency_samples;
        self.ttft_ms_total += other.ttft_ms_total;
        self.ttft_samples += other.ttft_samples;
    }
}

/// The measure groups are ranked by, largest first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RankBy {
    #[default]
    ListCostUsd,
    BilledCostUsd,
    TotalTokens,
    Requests,
}

impl RankBy {
    /// Largest first, so a sort by this puts the leader at the front.
    #[must_use]
    pub fn descending(self, left: &UsageMetrics, right: &UsageMetrics) -> Ordering {
        match self {
            Self::ListCostUsd => right.list_cost_usd.total_cmp(&left.list_cost_usd),
            Self::BilledCostUsd => right.billed_cost_usd.total_cmp(&left.billed_cost_usd),
            Self::TotalTokens => right.total_tokens.cmp(&left.total_tokens),
            Self::Requests => right.requests.cmp(&left.requests),
        }
    }
}
