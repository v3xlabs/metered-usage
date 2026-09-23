use std::sync::Arc;

use jiff::Timestamp;
use jiff::tz::Offset;
use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};

use crate::analytics::AnalyticsError;
use crate::analytics::breakdown::{Breakdown, BreakdownRow};
use crate::analytics::bucket::Bucket;
use crate::analytics::dimension::{Dimension, Dimensions, SourceRef};
use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::health::{AccountHealth, Health, HealthBucket, HealthWindow};
use crate::analytics::metrics::{RankBy, UsageMetrics};
use crate::analytics::series::{SeriesBucket, SeriesQuery};
use crate::analytics::session::SessionUsage;
use crate::analytics::summary::{DailyBurn, DistinctCounts, PeakDay, Summary};
use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::account::AccountSummaryOutput;
use crate::http::api::source::SourceKindOutput;
use crate::prelude::*;

const DEFAULT_TOP: usize = 8;
const MAX_TOP: usize = 50;
const DEFAULT_BREAKDOWN_LIMIT: usize = 50;
const MAX_BREAKDOWN_LIMIT: usize = 500;
const DEFAULT_SESSION_LIMIT: usize = 20;
const MAX_SESSION_LIMIT: usize = 200;
const SECONDS_PER_MINUTE: i32 = 60;
const DEFAULT_HEALTH_WINDOW_MINUTES: u32 = 60;
const DEFAULT_HEALTH_BUCKET_MINUTES: u32 = 5;

/// Every endpoint but `health` takes the same window and filters: `from` and `to` are
/// RFC 3339 with `to` exclusive and defaulting to now, and `from` absent starting at the
/// first matching event. `utc_offset_minutes` places day and week boundaries in the
/// viewer's local time.
/// Each filter takes comma separated values, any of which may match; different filters
/// must all match. The harness `unknown` matches events that name no harness.
pub struct AnalyticsApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl AnalyticsApi {
    /// Totals of the window, its rate per local day, its busiest days, and how many
    /// distinct values of each dimension it holds.
    #[oai(
        path = "/analytics/summary",
        method = "get",
        operation_id = "analytics_summary"
    )]
    async fn analytics_summary(
        &self,
        from: Query<Option<String>>,
        to: Query<Option<String>>,
        utc_offset_minutes: Query<Option<i32>>,
        #[oai(explode = false, default)] source_id: Query<Vec<String>>,
        #[oai(explode = false, default)] account_id: Query<Vec<String>>,
        #[oai(explode = false, default)] model: Query<Vec<String>>,
        #[oai(explode = false, default)] provider: Query<Vec<String>>,
        #[oai(explode = false, default)] harness: Query<Vec<String>>,
    ) -> SummaryResponse {
        let filter = match (Scope {
            from: from.0,
            to: to.0,
            utc_offset_minutes: utc_offset_minutes.0,
            source_id: source_id.0,
            account_id: account_id.0,
            model: model.0,
            provider: provider.0,
            harness: harness.0,
        })
        .try_into()
        {
            Ok(filter) => filter,
            Err(error) => return SummaryResponse::Invalid(Json(error)),
        };

        match Summary::load(&self.state.database, &filter).await {
            Ok(summary) => SummaryResponse::Found(Json(Box::new(summary.into()))),
            Err(error) => match refusal("analytics_summary", error) {
                Refusal::Invalid(error) => SummaryResponse::Invalid(Json(error)),
                Refusal::Failed(error) => SummaryResponse::Failed(Json(error)),
            },
        }
    }

    /// Usage per bucket of the viewer's local time. Every returned key has every bucket
    /// of the window, zero filled. Grouped, the `top` groups ranked over the whole window
    /// keep their key and the rest are merged under `other`; ungrouped, the key is `all`.
    #[oai(
        path = "/analytics/series",
        method = "get",
        operation_id = "analytics_series"
    )]
    async fn analytics_series(
        &self,
        from: Query<Option<String>>,
        to: Query<Option<String>>,
        utc_offset_minutes: Query<Option<i32>>,
        #[oai(explode = false, default)] source_id: Query<Vec<String>>,
        #[oai(explode = false, default)] account_id: Query<Vec<String>>,
        #[oai(explode = false, default)] model: Query<Vec<String>>,
        #[oai(explode = false, default)] provider: Query<Vec<String>>,
        #[oai(explode = false, default)] harness: Query<Vec<String>>,
        bucket: Query<BucketInput>,
        group_by: Query<Option<DimensionInput>>,
        top: Query<Option<usize>>,
        rank_by: Query<Option<RankByInput>>,
    ) -> SeriesResponse {
        let filter: AnalyticsFilter = match (Scope {
            from: from.0,
            to: to.0,
            utc_offset_minutes: utc_offset_minutes.0,
            source_id: source_id.0,
            account_id: account_id.0,
            model: model.0,
            provider: provider.0,
            harness: harness.0,
        })
        .try_into()
        {
            Ok(filter) => filter,
            Err(error) => return SeriesResponse::Invalid(Json(error)),
        };
        let query = SeriesQuery {
            bucket: bucket.0.into(),
            group_by: group_by.0.map(Dimension::from),
            top: top.0.unwrap_or(DEFAULT_TOP).clamp(1, MAX_TOP),
            rank_by: rank_by.0.map(RankBy::from).unwrap_or_default(),
        };

        match SeriesBucket::load(&self.state.database, &filter, query).await {
            Ok(buckets) => SeriesResponse::Found(Json(SeriesOutput {
                buckets: buckets
                    .into_iter()
                    .map(|bucket| SeriesBucketOutput {
                        start: bucket.start.display_with_offset(filter.offset).to_string(),
                        key: bucket.key,
                        metrics: bucket.metrics.into(),
                    })
                    .collect(),
            })),
            Err(error) => match refusal("analytics_series", error) {
                Refusal::Invalid(error) => SeriesResponse::Invalid(Json(error)),
                Refusal::Failed(error) => SeriesResponse::Failed(Json(error)),
            },
        }
    }

    /// Usage split by one dimension, largest first, with the total of every group.
    #[oai(
        path = "/analytics/breakdown",
        method = "get",
        operation_id = "analytics_breakdown"
    )]
    async fn analytics_breakdown(
        &self,
        from: Query<Option<String>>,
        to: Query<Option<String>>,
        utc_offset_minutes: Query<Option<i32>>,
        #[oai(explode = false, default)] source_id: Query<Vec<String>>,
        #[oai(explode = false, default)] account_id: Query<Vec<String>>,
        #[oai(explode = false, default)] model: Query<Vec<String>>,
        #[oai(explode = false, default)] provider: Query<Vec<String>>,
        #[oai(explode = false, default)] harness: Query<Vec<String>>,
        group_by: Query<DimensionInput>,
        rank_by: Query<Option<RankByInput>>,
        limit: Query<Option<usize>>,
    ) -> BreakdownResponse {
        let filter = match (Scope {
            from: from.0,
            to: to.0,
            utc_offset_minutes: utc_offset_minutes.0,
            source_id: source_id.0,
            account_id: account_id.0,
            model: model.0,
            provider: provider.0,
            harness: harness.0,
        })
        .try_into()
        {
            Ok(filter) => filter,
            Err(error) => return BreakdownResponse::Invalid(Json(error)),
        };

        match Breakdown::load(
            &self.state.database,
            &filter,
            group_by.0.into(),
            rank_by.0.map(RankBy::from).unwrap_or_default(),
            limit
                .0
                .unwrap_or(DEFAULT_BREAKDOWN_LIMIT)
                .clamp(1, MAX_BREAKDOWN_LIMIT),
        )
        .await
        {
            Ok(breakdown) => BreakdownResponse::Found(Json(breakdown.into())),
            Err(error) => match refusal("analytics_breakdown", error) {
                Refusal::Invalid(error) => BreakdownResponse::Invalid(Json(error)),
                Refusal::Failed(error) => BreakdownResponse::Failed(Json(error)),
            },
        }
    }

    /// Agent sessions, largest first. Events without a session are left out; a session
    /// that used several accounts reports the one that carried most of its requests.
    #[oai(
        path = "/analytics/sessions",
        method = "get",
        operation_id = "analytics_sessions"
    )]
    async fn analytics_sessions(
        &self,
        from: Query<Option<String>>,
        to: Query<Option<String>>,
        utc_offset_minutes: Query<Option<i32>>,
        #[oai(explode = false, default)] source_id: Query<Vec<String>>,
        #[oai(explode = false, default)] account_id: Query<Vec<String>>,
        #[oai(explode = false, default)] model: Query<Vec<String>>,
        #[oai(explode = false, default)] provider: Query<Vec<String>>,
        #[oai(explode = false, default)] harness: Query<Vec<String>>,
        rank_by: Query<Option<RankByInput>>,
        limit: Query<Option<usize>>,
    ) -> SessionsResponse {
        let filter = match (Scope {
            from: from.0,
            to: to.0,
            utc_offset_minutes: utc_offset_minutes.0,
            source_id: source_id.0,
            account_id: account_id.0,
            model: model.0,
            provider: provider.0,
            harness: harness.0,
        })
        .try_into()
        {
            Ok(filter) => filter,
            Err(error) => return SessionsResponse::Invalid(Json(error)),
        };

        match SessionUsage::list(
            &self.state.database,
            &filter,
            rank_by.0.map(RankBy::from).unwrap_or_default(),
            limit
                .0
                .unwrap_or(DEFAULT_SESSION_LIMIT)
                .clamp(1, MAX_SESSION_LIMIT),
        )
        .await
        {
            Ok(sessions) => SessionsResponse::Found(Json(SessionListOutput {
                sessions: sessions.into_iter().map(SessionRowOutput::from).collect(),
            })),
            Err(error) => match refusal("analytics_sessions", error) {
                Refusal::Invalid(error) => SessionsResponse::Invalid(Json(error)),
                Refusal::Failed(error) => SessionsResponse::Failed(Json(error)),
            },
        }
    }

    /// The values of each filter that at least one event in the window has.
    #[oai(
        path = "/analytics/dimensions",
        method = "get",
        operation_id = "analytics_dimensions"
    )]
    async fn analytics_dimensions(
        &self,
        from: Query<Option<String>>,
        to: Query<Option<String>>,
        utc_offset_minutes: Query<Option<i32>>,
        #[oai(explode = false, default)] source_id: Query<Vec<String>>,
        #[oai(explode = false, default)] account_id: Query<Vec<String>>,
        #[oai(explode = false, default)] model: Query<Vec<String>>,
        #[oai(explode = false, default)] provider: Query<Vec<String>>,
        #[oai(explode = false, default)] harness: Query<Vec<String>>,
    ) -> DimensionsResponse {
        let filter = match (Scope {
            from: from.0,
            to: to.0,
            utc_offset_minutes: utc_offset_minutes.0,
            source_id: source_id.0,
            account_id: account_id.0,
            model: model.0,
            provider: provider.0,
            harness: harness.0,
        })
        .try_into()
        {
            Ok(filter) => filter,
            Err(error) => return DimensionsResponse::Invalid(Json(error)),
        };

        match Dimensions::load(&self.state.database, &filter).await {
            Ok(dimensions) => DimensionsResponse::Found(Json(dimensions.into())),
            Err(error) => match refusal("analytics_dimensions", error) {
                Refusal::Invalid(error) => DimensionsResponse::Invalid(Json(error)),
                Refusal::Failed(error) => DimensionsResponse::Failed(Json(error)),
            },
        }
    }

    /// Requests and failures per account in the last `window_minutes`, cut into buckets
    /// of `bucket_minutes` aligned to whole multiples of it since the Unix epoch. Every
    /// account has every bucket, oldest first, the last one holding now. An account with
    /// no request in the window is present only when `account_id` names it.
    #[oai(
        path = "/analytics/health",
        method = "get",
        operation_id = "analytics_health"
    )]
    async fn analytics_health(
        &self,
        window_minutes: Query<Option<u32>>,
        bucket_minutes: Query<Option<u32>>,
        #[oai(explode = false, default)] source_id: Query<Vec<String>>,
        #[oai(explode = false, default)] account_id: Query<Vec<String>>,
    ) -> HealthResponse {
        let window = match HealthWindow::new(
            window_minutes.0.unwrap_or(DEFAULT_HEALTH_WINDOW_MINUTES),
            bucket_minutes.0.unwrap_or(DEFAULT_HEALTH_BUCKET_MINUTES),
        ) {
            Ok(window) => window,
            Err(error) => return HealthResponse::Invalid(Json(message(&error.to_string()))),
        };
        let (source_ids, account_ids) = match (
            ids(source_id.0, "source_id must be source ids"),
            ids(account_id.0, "account_id must be account ids"),
        ) {
            (Ok(source_ids), Ok(account_ids)) => (source_ids, account_ids),
            (Err(error), _) | (_, Err(error)) => return HealthResponse::Invalid(Json(error)),
        };

        match Health::load(
            &self.state.database,
            window,
            Timestamp::now(),
            source_ids,
            account_ids,
        )
        .await
        {
            Ok(health) => HealthResponse::Found(Json(health.into())),
            Err(error) => match refusal("analytics_health", error) {
                Refusal::Invalid(error) => HealthResponse::Invalid(Json(error)),
                Refusal::Failed(error) => HealthResponse::Failed(Json(error)),
            },
        }
    }
}

/// The window and filters as they arrived, before anything was read out of them.
struct Scope {
    from: Option<String>,
    to: Option<String>,
    utc_offset_minutes: Option<i32>,
    source_id: Vec<String>,
    account_id: Vec<String>,
    model: Vec<String>,
    provider: Vec<String>,
    harness: Vec<String>,
}

impl TryFrom<Scope> for AnalyticsFilter {
    type Error = Error;

    fn try_from(scope: Scope) -> Result<Self, Self::Error> {
        let offset = scope
            .utc_offset_minutes
            .unwrap_or(0)
            .checked_mul(SECONDS_PER_MINUTE)
            .and_then(|seconds| Offset::from_seconds(seconds).ok())
            .ok_or_else(|| message("utc_offset_minutes is outside the supported range"))?;

        Ok(Self {
            source_ids: ids(scope.source_id, "source_id must be source ids")?,
            account_ids: ids(scope.account_id, "account_id must be account ids")?,
            models: values(scope.model),
            providers: values(scope.provider),
            harnesses: values(scope.harness),
            ..Self::window(
                instant(scope.from.as_deref(), "from")?,
                instant(scope.to.as_deref(), "to")?,
                offset,
            )
        })
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "Dimension", rename_all = "snake_case")]
enum DimensionInput {
    Model,
    Provider,
    Account,
    Harness,
    Source,
    Session,
}

impl From<DimensionInput> for Dimension {
    fn from(dimension: DimensionInput) -> Self {
        match dimension {
            DimensionInput::Model => Self::Model,
            DimensionInput::Provider => Self::Provider,
            DimensionInput::Account => Self::Account,
            DimensionInput::Harness => Self::Harness,
            DimensionInput::Source => Self::Source,
            DimensionInput::Session => Self::Session,
        }
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "RankBy", rename_all = "snake_case")]
enum RankByInput {
    ListCostUsd,
    BilledCostUsd,
    TotalTokens,
    Requests,
}

impl From<RankByInput> for RankBy {
    fn from(rank_by: RankByInput) -> Self {
        match rank_by {
            RankByInput::ListCostUsd => Self::ListCostUsd,
            RankByInput::BilledCostUsd => Self::BilledCostUsd,
            RankByInput::TotalTokens => Self::TotalTokens,
            RankByInput::Requests => Self::Requests,
        }
    }
}

/// A week starts on Monday.
#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "Bucket", rename_all = "snake_case")]
enum BucketInput {
    Hour,
    Day,
    Week,
}

impl From<BucketInput> for Bucket {
    fn from(bucket: BucketInput) -> Self {
        match bucket {
            BucketInput::Hour => Self::Hour,
            BucketInput::Day => Self::Day,
            BucketInput::Week => Self::Week,
        }
    }
}

/// Uncached input is input less cache reads and writes, never below zero. The four cost
/// parts are the list cost of uncached input, cache reads, cache writes and output (with
/// its reasoning), each at the event's own price, and add up to `list_cost_usd`. Cache
/// savings are what cache reads saved against the input rate, less what cache writes cost
/// above it, at each event's own price; negative when writes outweighed reads.
#[derive(Debug, Object)]
#[oai(rename = "UsageMetrics", skip_serializing_if_is_none)]
struct UsageMetricsOutput {
    requests: i64,
    failures: i64,
    input_tokens: i64,
    uncached_input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    unclassified_tokens: i64,
    total_tokens: i64,
    list_cost_usd: f64,
    billed_cost_usd: f64,
    uncached_input_cost_usd: f64,
    cache_read_cost_usd: f64,
    cache_write_cost_usd: f64,
    output_cost_usd: f64,
    cache_savings_usd: f64,
    /// Events no price has been found for yet, left out of every cost.
    unpriced_requests: i64,
    avg_latency_ms: Option<f64>,
    avg_ttft_ms: Option<f64>,
}

impl From<UsageMetrics> for UsageMetricsOutput {
    fn from(metrics: UsageMetrics) -> Self {
        Self {
            requests: metrics.requests,
            failures: metrics.failures,
            input_tokens: metrics.input_tokens,
            uncached_input_tokens: metrics.uncached_input_tokens,
            cache_read_tokens: metrics.cache_read_tokens,
            cache_write_tokens: metrics.cache_write_tokens,
            output_tokens: metrics.output_tokens,
            reasoning_tokens: metrics.reasoning_tokens,
            unclassified_tokens: metrics.unclassified_tokens,
            total_tokens: metrics.total_tokens,
            list_cost_usd: metrics.list_cost_usd,
            billed_cost_usd: metrics.billed_cost_usd,
            uncached_input_cost_usd: metrics.uncached_input_cost_usd,
            cache_read_cost_usd: metrics.cache_read_cost_usd,
            cache_write_cost_usd: metrics.cache_write_cost_usd,
            output_cost_usd: metrics.output_cost_usd,
            cache_savings_usd: metrics.cache_savings_usd,
            unpriced_requests: metrics.unpriced_requests,
            avg_latency_ms: metrics.avg_latency_ms(),
            avg_ttft_ms: metrics.avg_ttft_ms(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Summary", skip_serializing_if_is_none)]
struct SummaryOutput {
    metrics: UsageMetricsOutput,
    /// Local days from the window's first day to the day of its last instant.
    range_days: i32,
    /// Local days with at least one event.
    active_days: i64,
    daily_burn: DailyBurnOutput,
    peak_day_by_cost: Option<PeakDayOutput>,
    peak_day_by_tokens: Option<PeakDayOutput>,
    /// Cache reads over input; absent when there was no input.
    cache_hit_rate: Option<f64>,
    distinct: DistinctCountsOutput,
}

impl From<Summary> for SummaryOutput {
    fn from(summary: Summary) -> Self {
        Self {
            metrics: summary.metrics.into(),
            range_days: summary.range_days,
            active_days: summary.active_days,
            daily_burn: summary.daily_burn.into(),
            peak_day_by_cost: summary.peak_day_by_cost.map(PeakDayOutput::from),
            peak_day_by_tokens: summary.peak_day_by_tokens.map(PeakDayOutput::from),
            cache_hit_rate: summary.cache_hit_rate,
            distinct: summary.distinct.into(),
        }
    }
}

/// The window's totals divided by `range_days`.
#[derive(Debug, Object)]
#[oai(rename = "DailyBurn")]
struct DailyBurnOutput {
    list_cost_usd: f64,
    billed_cost_usd: f64,
    total_tokens: i64,
}

impl From<DailyBurn> for DailyBurnOutput {
    fn from(burn: DailyBurn) -> Self {
        Self {
            list_cost_usd: burn.list_cost_usd,
            billed_cost_usd: burn.billed_cost_usd,
            total_tokens: burn.total_tokens,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "PeakDay")]
struct PeakDayOutput {
    /// `YYYY-MM-DD` in the viewer's local time.
    day: String,
    list_cost_usd: f64,
    total_tokens: i64,
}

impl From<PeakDay> for PeakDayOutput {
    fn from(peak: PeakDay) -> Self {
        Self {
            day: peak.day.to_string(),
            list_cost_usd: peak.list_cost_usd,
            total_tokens: peak.total_tokens,
        }
    }
}

/// Events without a session are not a session; events without a harness count as the
/// harness `unknown`.
#[derive(Debug, Object)]
#[oai(rename = "DistinctCounts")]
struct DistinctCountsOutput {
    models: i64,
    providers: i64,
    accounts: i64,
    harnesses: i64,
    sources: i64,
    sessions: i64,
}

impl From<DistinctCounts> for DistinctCountsOutput {
    fn from(distinct: DistinctCounts) -> Self {
        Self {
            models: distinct.models,
            providers: distinct.providers,
            accounts: distinct.accounts,
            harnesses: distinct.harnesses,
            sources: distinct.sources,
            sessions: distinct.sessions,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Series")]
struct SeriesOutput {
    buckets: Vec<SeriesBucketOutput>,
}

#[derive(Debug, Object)]
#[oai(rename = "SeriesBucket")]
struct SeriesBucketOutput {
    /// RFC 3339 at the viewer's offset.
    start: String,
    key: String,
    metrics: UsageMetricsOutput,
}

#[derive(Debug, Object)]
#[oai(rename = "Breakdown")]
struct BreakdownOutput {
    rows: Vec<BreakdownRowOutput>,
    /// Every group, including those past the limit.
    total: UsageMetricsOutput,
}

impl From<Breakdown> for BreakdownOutput {
    fn from(breakdown: Breakdown) -> Self {
        Self {
            rows: breakdown
                .rows
                .into_iter()
                .map(BreakdownRowOutput::from)
                .collect(),
            total: breakdown.total.into(),
        }
    }
}

/// An account or a source is keyed by its id; an absent harness or session by `unknown`.
#[derive(Debug, Object)]
#[oai(rename = "BreakdownRow")]
struct BreakdownRowOutput {
    key: String,
    metrics: UsageMetricsOutput,
}

impl From<BreakdownRow> for BreakdownRowOutput {
    fn from(row: BreakdownRow) -> Self {
        Self {
            key: row.key,
            metrics: row.metrics.into(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "SessionList")]
struct SessionListOutput {
    sessions: Vec<SessionRowOutput>,
}

#[derive(Debug, Object)]
#[oai(rename = "SessionRow", skip_serializing_if_is_none)]
struct SessionRowOutput {
    session_id: String,
    source_id: String,
    account: AccountSummaryOutput,
    harness: Option<String>,
    models: Vec<String>,
    first_at: String,
    last_at: String,
    metrics: UsageMetricsOutput,
}

impl From<SessionUsage> for SessionRowOutput {
    fn from(session: SessionUsage) -> Self {
        Self {
            session_id: session.session_id,
            source_id: session.source_id.encode(),
            account: session.account.into(),
            harness: session.harness,
            models: session.models,
            first_at: session.first_at.to_string(),
            last_at: session.last_at.to_string(),
            metrics: session.metrics.into(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Dimensions")]
struct DimensionsOutput {
    models: Vec<String>,
    providers: Vec<String>,
    /// `unknown` stands for events that name no harness.
    harnesses: Vec<String>,
    accounts: Vec<AccountSummaryOutput>,
    sources: Vec<SourceRefOutput>,
}

impl From<Dimensions> for DimensionsOutput {
    fn from(dimensions: Dimensions) -> Self {
        Self {
            models: dimensions.models,
            providers: dimensions.providers,
            harnesses: dimensions.harnesses,
            accounts: dimensions
                .accounts
                .into_iter()
                .map(AccountSummaryOutput::from)
                .collect(),
            sources: dimensions
                .sources
                .into_iter()
                .map(SourceRefOutput::from)
                .collect(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "SourceRef")]
struct SourceRefOutput {
    source_id: String,
    name: String,
    kind: SourceKindOutput,
}

impl From<SourceRef> for SourceRefOutput {
    fn from(source: SourceRef) -> Self {
        Self {
            source_id: source.id.encode(),
            name: source.name,
            kind: source.kind.into(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "AccountHealthStrip")]
struct HealthOutput {
    bucket_minutes: u32,
    /// RFC 3339 in UTC, the start of the oldest bucket.
    buckets_start: String,
    accounts: Vec<AccountHealthOutput>,
}

impl From<Health> for HealthOutput {
    fn from(health: Health) -> Self {
        Self {
            bucket_minutes: health.bucket_minutes,
            buckets_start: health.buckets_start.to_string(),
            accounts: health
                .accounts
                .into_iter()
                .map(AccountHealthOutput::from)
                .collect(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "AccountHealth")]
struct AccountHealthOutput {
    account_id: String,
    buckets: Vec<HealthBucketOutput>,
}

impl From<AccountHealth> for AccountHealthOutput {
    fn from(account: AccountHealth) -> Self {
        Self {
            account_id: account.account_id.encode(),
            buckets: account
                .buckets
                .into_iter()
                .map(HealthBucketOutput::from)
                .collect(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "HealthBucket")]
struct HealthBucketOutput {
    /// RFC 3339 in UTC.
    start: String,
    requests: i64,
    failures: i64,
}

impl From<HealthBucket> for HealthBucketOutput {
    fn from(bucket: HealthBucket) -> Self {
        Self {
            start: bucket.start.to_string(),
            requests: bucket.requests,
            failures: bucket.failures,
        }
    }
}

#[derive(ApiResponse)]
enum HealthResponse {
    #[oai(status = 200)]
    Found(Json<HealthOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum SummaryResponse {
    #[oai(status = 200)]
    Found(Json<Box<SummaryOutput>>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum SeriesResponse {
    #[oai(status = 200)]
    Found(Json<SeriesOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum BreakdownResponse {
    #[oai(status = 200)]
    Found(Json<BreakdownOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum SessionsResponse {
    #[oai(status = 200)]
    Found(Json<SessionListOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum DimensionsResponse {
    #[oai(status = 200)]
    Found(Json<DimensionsOutput>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

enum Refusal {
    Invalid(Error),
    Failed(Error),
}

fn refusal(operation: &'static str, error: AnalyticsError) -> Refusal {
    match error {
        AnalyticsError::Database(error) => {
            tracing::error!(%operation, %error, "analytics query failed");
            Refusal::Failed(message("the database refused the request"))
        }
        AnalyticsError::UnknownAccount(_)
        | AnalyticsError::UnknownSource(_)
        | AnalyticsError::TooManyBuckets
        | AnalyticsError::Window(_) => Refusal::Invalid(message(&error.to_string())),
    }
}

fn ids<T>(values: Vec<String>, refusal: &'static str) -> Result<Vec<Id<T>>, Error> {
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(|value| value.parse().map_err(|_| message(refusal)))
        .collect()
}

fn values(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect()
}

fn instant(value: Option<&str>, name: &'static str) -> Result<Option<Timestamp>, Error> {
    value
        .map(str::parse::<Timestamp>)
        .transpose()
        .map_err(|_| message(&format!("{name} must be an RFC 3339 timestamp")))
}

fn message(text: &str) -> Error {
    Error {
        message: text.to_owned(),
    }
}
