use std::sync::Arc;

use jiff::Timestamp;
use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};

use crate::analytics::AnalyticsError;
use crate::analytics::bucket::Bucket;
use crate::analytics::dimension::Dimension;
use crate::analytics::metrics::UsageMetrics;
use crate::analytics::rollup::{Rollup, RollupBucket, RollupGroup};
use crate::app::AppState;
use crate::http::api::Error;
use crate::http::conditional::conditional;

pub struct UsageSummaryApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl UsageSummaryApi {
    /// Calls, tokens and costs of the window `[from, to)`, in UTC hours or days.
    ///
    /// `from` and `to` are required RFC 3339 instants, `to` exclusive and after `from`.
    /// `step` is `hour` or `day`: buckets start on whole UTC hours or at UTC midnight, and a
    /// bucket that `from` or `to` cuts counts only the part inside the window. Every bucket
    /// that overlaps the window is listed, oldest first, with zeros where nothing happened;
    /// a window of more than 5000 buckets is refused.
    ///
    /// `group` is `account`, `model` or `provider`, and absent sums everything. Grouped,
    /// `groups` holds every group with usage in the window and each bucket holds every
    /// group with usage in that bucket, both ordered by key. No group is dropped or merged,
    /// so the groups of a bucket add up to its `total`. An account is keyed by its
    /// `account_id`, as `/accounts` lists it. Ungrouped, neither `groups` is present.
    ///
    /// Aggregation is not anonymization. Narrow windows can isolate a call's token counts
    /// and costs, and repeated queries can infer its timestamp. The answer does not include
    /// request bodies, request ids, caller, session, harness or user agent.
    ///
    /// The answer carries an `ETag` of its bytes. Sent back in `If-None-Match`, it answers 304
    /// with no body while those bytes are the same, which holds until a `usage.updated`.
    #[oai(
        path = "/usage/summary",
        method = "get",
        operation_id = "usage_summary",
        transform = "conditional",
        request_header(
            name = "If-None-Match",
            ty = "Option<String>",
            description = "The ETag of an earlier answer. While the answer is the same, it is 304 with no body."
        ),
        response_header(
            name = "ETag",
            ty = "Option<String>",
            description = "A tag of the answer's bytes, to send back in If-None-Match. Absent on an error."
        )
    )]
    async fn usage_summary(
        &self,
        from: Query<String>,
        to: Query<String>,
        step: Query<StepInput>,
        group: Query<Option<GroupInput>>,
    ) -> UsageSummaryResponse {
        let (from, to) = match (instant(&from.0, "from"), instant(&to.0, "to")) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(error), _) | (_, Err(error)) => {
                return UsageSummaryResponse::Invalid(Json(error));
            }
        };
        if from >= to {
            return UsageSummaryResponse::Invalid(Json(message("to must be after from")));
        }

        match Rollup::load(
            &self.state.database,
            from,
            to,
            step.0.into(),
            group.0.map(Dimension::from),
        )
        .await
        {
            Ok(rollup) => UsageSummaryResponse::Found(Json(rollup.into())),
            Err(AnalyticsError::Database(error)) => {
                tracing::error!(operation = "usage_summary", %error, "usage summary query failed");
                UsageSummaryResponse::Failed(Json(message("the database refused the request")))
            }
            Err(
                error @ (AnalyticsError::UnknownAccount(_)
                | AnalyticsError::UnknownSource(_)
                | AnalyticsError::TooManyBuckets
                | AnalyticsError::Window(_)),
            ) => UsageSummaryResponse::Invalid(Json(message(&error.to_string()))),
        }
    }
}

/// How long each bucket is, in UTC.
#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "UsageSummaryStep", rename_all = "snake_case")]
enum StepInput {
    Hour,
    Day,
}

impl From<StepInput> for Bucket {
    fn from(step: StepInput) -> Self {
        match step {
            StepInput::Hour => Self::Hour,
            StepInput::Day => Self::Day,
        }
    }
}

/// What the usage is split by.
#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "UsageSummaryGroupBy", rename_all = "snake_case")]
enum GroupInput {
    Account,
    Model,
    Provider,
}

impl From<GroupInput> for Dimension {
    fn from(group: GroupInput) -> Self {
        match group {
            GroupInput::Account => Self::Account,
            GroupInput::Model => Self::Model,
            GroupInput::Provider => Self::Provider,
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "UsageSummary", skip_serializing_if_is_none)]
struct UsageSummaryOutput {
    /// The whole window.
    total: UsageSummaryMetricsOutput,
    /// Every group with usage in the window, by key. Present only with `group`.
    groups: Option<Vec<UsageSummaryGroupOutput>>,
    /// Every bucket that overlaps the window, oldest first.
    buckets: Vec<UsageSummaryBucketOutput>,
}

impl From<Rollup> for UsageSummaryOutput {
    fn from(rollup: Rollup) -> Self {
        Self {
            total: rollup.total.into(),
            groups: rollup.groups.map(groups),
            buckets: rollup
                .buckets
                .into_iter()
                .map(UsageSummaryBucketOutput::from)
                .collect(),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "UsageSummaryBucket", skip_serializing_if_is_none)]
struct UsageSummaryBucketOutput {
    /// RFC 3339 in UTC: a whole hour, or midnight.
    start: String,
    total: UsageSummaryMetricsOutput,
    /// Every group with usage in this bucket, by key. Present only with `group`.
    groups: Option<Vec<UsageSummaryGroupOutput>>,
}

impl From<RollupBucket> for UsageSummaryBucketOutput {
    fn from(bucket: RollupBucket) -> Self {
        Self {
            start: bucket.start.to_string(),
            total: bucket.total.into(),
            groups: bucket.groups.map(groups),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "UsageSummaryGroup")]
struct UsageSummaryGroupOutput {
    /// An `account_id`, a model or a provider, as `group` asked.
    key: String,
    metrics: UsageSummaryMetricsOutput,
}

impl From<RollupGroup> for UsageSummaryGroupOutput {
    fn from(group: RollupGroup) -> Self {
        Self {
            key: group.key,
            metrics: group.metrics.into(),
        }
    }
}

/// Costs are the ones stored on each call when it was priced, never a guess. A call no
/// price has been found for adds nothing to `list_cost_usd` and is counted in
/// `unpriced_requests`; a call with neither a price nor a charge its gateway reported adds
/// nothing to `billed_cost_usd` and is counted in `unbilled_requests`. A cost is complete
/// when its count is 0.
#[derive(Debug, Object)]
#[oai(rename = "UsageSummaryMetrics")]
struct UsageSummaryMetricsOutput {
    /// Calls, failed ones included.
    requests: i64,
    /// Every input token, cache reads and writes included.
    input_tokens: i64,
    /// Input less cache reads and writes, never below zero for a call.
    uncached_input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    /// Every output token, reasoning included.
    output_tokens: i64,
    /// The part of the output spent reasoning.
    reasoning_tokens: i64,
    /// Tokens the gateway counted in a total without saying whether they were input or
    /// output.
    unclassified_tokens: i64,
    total_tokens: i64,
    /// At list price, of the calls that have one.
    list_cost_usd: f64,
    /// What was charged: what the gateway reported, else 0 for an account signed in with
    /// OAuth, else the list price.
    billed_cost_usd: f64,
    /// Calls left out of `list_cost_usd`.
    unpriced_requests: i64,
    /// Calls left out of `billed_cost_usd`.
    unbilled_requests: i64,
}

impl From<UsageMetrics> for UsageSummaryMetricsOutput {
    fn from(metrics: UsageMetrics) -> Self {
        Self {
            requests: metrics.requests,
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
            unpriced_requests: metrics.unpriced_requests,
            unbilled_requests: metrics.unbilled_requests,
        }
    }
}

#[derive(ApiResponse)]
enum UsageSummaryResponse {
    #[oai(status = 200)]
    Found(Json<UsageSummaryOutput>),
    /// A bound that is not RFC 3339, `to` not after `from`, or too many buckets.
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn groups(groups: Vec<RollupGroup>) -> Vec<UsageSummaryGroupOutput> {
    groups
        .into_iter()
        .map(UsageSummaryGroupOutput::from)
        .collect()
}

fn instant(value: &str, name: &'static str) -> Result<Timestamp, Error> {
    value
        .parse()
        .map_err(|_| message(&format!("{name} must be an RFC 3339 timestamp")))
}

fn message(text: &str) -> Error {
    Error {
        message: text.to_owned(),
    }
}
