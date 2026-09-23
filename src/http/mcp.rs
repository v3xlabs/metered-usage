use std::sync::Arc;

use jiff::Timestamp;
use jiff::tz::Offset;
use poem::http::{HeaderValue, header};
use poem::{EndpointExt, IntoEndpoint, Request};
use poem_mcpserver::{McpServer, Tools, streamable_http, tool::StructuredContent};
use schemars::JsonSchema;
use serde::Serialize;

use crate::analytics::AnalyticsError;
use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::metrics::UsageMetrics;
use crate::analytics::summary::{PeakDay, Summary};
use crate::app::AppState;
use crate::http::api::leverage::DEFAULT_PERIODS as DEFAULT_LEVERAGE_PERIODS;
use crate::plan::leverage::PlanLeverage;
use crate::prelude::*;
use crate::quota::credential::QuotaAccount;

const EVENT_STREAM: &str = "text/event-stream";
const DEFAULT_EVENTS: i64 = 50;
const MAX_EVENTS: i64 = 500;

#[derive(Debug, Serialize, JsonSchema)]
struct SourcesOutput {
    sources: Vec<SourceOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct SourceOutput {
    source_id: String,
    name: String,
    kind: String,
    base_url: String,
    enabled: bool,
    created_at: String,
    last_event_at: Option<String>,
    event_count: i64,
}

/// Totals of a window and its rate per local day. The four `*_cost_usd` parts of the list
/// cost add up to `list_cost_usd`. `cache_savings_usd` is what cache reads saved against
/// the input rate, less what cache writes cost above it; it can be negative.
#[derive(Debug, Serialize, JsonSchema)]
struct SummaryOutput {
    metrics: MetricsOutput,
    range_days: i32,
    active_days: i64,
    daily_burn: DailyBurnOutput,
    peak_day_by_cost: Option<PeakDayOutput>,
    peak_day_by_tokens: Option<PeakDayOutput>,
    cache_hit_rate: Option<f64>,
    distinct: DistinctOutput,
}

impl From<Summary> for SummaryOutput {
    fn from(summary: Summary) -> Self {
        Self {
            metrics: summary.metrics.into(),
            range_days: summary.range_days,
            active_days: summary.active_days,
            daily_burn: DailyBurnOutput {
                list_cost_usd: summary.daily_burn.list_cost_usd,
                billed_cost_usd: summary.daily_burn.billed_cost_usd,
                total_tokens: summary.daily_burn.total_tokens,
            },
            peak_day_by_cost: summary.peak_day_by_cost.map(PeakDayOutput::from),
            peak_day_by_tokens: summary.peak_day_by_tokens.map(PeakDayOutput::from),
            cache_hit_rate: summary.cache_hit_rate,
            distinct: DistinctOutput {
                models: summary.distinct.models,
                providers: summary.distinct.providers,
                accounts: summary.distinct.accounts,
                harnesses: summary.distinct.harnesses,
                sources: summary.distinct.sources,
                sessions: summary.distinct.sessions,
            },
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
struct MetricsOutput {
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
    unpriced_requests: i64,
    avg_latency_ms: Option<f64>,
    avg_ttft_ms: Option<f64>,
}

impl From<UsageMetrics> for MetricsOutput {
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

#[derive(Debug, Serialize, JsonSchema)]
struct DailyBurnOutput {
    list_cost_usd: f64,
    billed_cost_usd: f64,
    total_tokens: i64,
}

/// `day` is `YYYY-MM-DD` at the requested offset.
#[derive(Debug, Serialize, JsonSchema)]
struct PeakDayOutput {
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

#[derive(Debug, Serialize, JsonSchema)]
struct DistinctOutput {
    models: i64,
    providers: i64,
    accounts: i64,
    harnesses: i64,
    sources: i64,
    sessions: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct AccountsOutput {
    accounts: Vec<AccountOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct AccountOutput {
    #[serde(flatten)]
    summary: AccountSummaryOutput,
    first_seen_at: String,
    last_seen_at: String,
    event_count: i64,
}

impl From<Account> for AccountOutput {
    fn from(account: Account) -> Self {
        Self {
            summary: account.summary.into(),
            first_seen_at: account.first_seen_at.to_string(),
            last_seen_at: account.last_seen_at.to_string(),
            event_count: account.event_count,
        }
    }
}

/// An upstream credential. `label` is the login email for OAuth and only the last four
/// characters of anything else.
#[derive(Debug, Serialize, JsonSchema)]
struct AccountSummaryOutput {
    account_id: String,
    source_id: String,
    source_name: String,
    provider: String,
    auth_kind: String,
    label: Option<String>,
    display_name: Option<String>,
}

impl From<AccountSummary> for AccountSummaryOutput {
    fn from(summary: AccountSummary) -> Self {
        Self {
            account_id: summary.id.encode(),
            source_id: summary.source_id.encode(),
            source_name: summary.source_name,
            provider: summary.provider,
            auth_kind: summary.auth_kind.stored().to_owned(),
            label: summary.label,
            display_name: summary.display_name,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
struct EventsOutput {
    events: Vec<EventOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EventOutput {
    event_id: String,
    source_id: String,
    occurred_at: String,
    provider: String,
    model: String,
    account: AccountSummaryOutput,
    harness: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    status_code: i64,
    failed: bool,
    error_message: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct QuotaStatusOutput {
    accounts: Vec<QuotaAccountOutput>,
}

/// A CLIProxy credential's last known health and quota. `next_retry_after` is CLIProxy's
/// scheduling decision, not the provider's reset; a window's `resets_at` is the provider's.
#[derive(Debug, Serialize, JsonSchema)]
struct QuotaAccountOutput {
    account: AccountSummaryOutput,
    status: Option<String>,
    status_message: Option<String>,
    disabled: bool,
    unavailable: bool,
    next_retry_after: Option<String>,
    cooldowns: Vec<CooldownOutput>,
    account_type: Option<String>,
    plan: Option<String>,
    soft_observed_at: Option<String>,
    hard_refreshed_at: Option<String>,
    hard_refresh_error: Option<String>,
    hard_refreshable: bool,
    windows: Vec<QuotaWindowOutput>,
}

impl From<QuotaAccount> for QuotaAccountOutput {
    fn from(account: QuotaAccount) -> Self {
        Self {
            account: account.account.into(),
            status: account.status,
            status_message: account.status_message,
            disabled: account.disabled,
            unavailable: account.unavailable,
            next_retry_after: account.next_retry_after.map(|at| at.0.to_string()),
            cooldowns: account
                .cooldowns
                .0
                .into_iter()
                .map(|cooldown| CooldownOutput {
                    scope: cooldown.scope,
                    model_key: cooldown.model_key,
                    reason: cooldown.reason,
                    retry_at: cooldown.retry_at.to_string(),
                    remaining_seconds: cooldown.remaining_seconds,
                })
                .collect(),
            account_type: account.account_type,
            plan: account.plan,
            soft_observed_at: account.soft_observed_at.map(|at| at.0.to_string()),
            hard_refreshed_at: account.hard_refreshed_at.map(|at| at.0.to_string()),
            hard_refresh_error: account.hard_refresh_error,
            hard_refreshable: account.hard_refreshable,
            windows: account
                .windows
                .into_iter()
                .map(|window| QuotaWindowOutput {
                    window_key: window.window_key,
                    label: window.label,
                    used_fraction: window.used_fraction,
                    used_value: window.used_value,
                    limit_value: window.limit_value,
                    unit: window.unit,
                    window_seconds: window.window_seconds,
                    resets_at: window.resets_at.map(|at| at.0.to_string()),
                    observed_at: window.observed_at.to_string(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
struct CooldownOutput {
    scope: String,
    model_key: Option<String>,
    reason: String,
    retry_at: String,
    remaining_seconds: i64,
}

/// `used_fraction` is 0 to 1 whatever unit the provider counts in.
#[derive(Debug, Serialize, JsonSchema)]
struct QuotaWindowOutput {
    window_key: String,
    label: String,
    used_fraction: Option<f64>,
    used_value: Option<f64>,
    limit_value: Option<f64>,
    unit: Option<String>,
    window_seconds: Option<i64>,
    resets_at: Option<String>,
    observed_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct LeverageOutput {
    plans: Vec<PlanLeverageOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct PlanLeverageOutput {
    plan: PlanOutput,
    /// Newest first.
    periods: Vec<LeveragePeriodOutput>,
}

impl From<PlanLeverage> for PlanLeverageOutput {
    fn from(leverage: PlanLeverage) -> Self {
        let plan = leverage.plan;
        Self {
            plan: PlanOutput {
                plan_id: plan.id.encode(),
                account: plan.account.into(),
                name: plan.name,
                monthly_usd: plan.monthly_usd,
                period_start: plan.period_start.to_string(),
                period_end: plan.period_end.map(|end| end.0.to_string()),
            },
            periods: leverage
                .periods
                .into_iter()
                .map(|period| LeveragePeriodOutput {
                    period_start: period.start.to_string(),
                    period_end: period.end.to_string(),
                    complete: period.complete,
                    requests: period.requests,
                    total_tokens: period.total_tokens,
                    list_cost_usd: period.list_cost_usd,
                    leverage: period.leverage,
                    effective_usd_per_mtok: period.effective_usd_per_mtok,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
struct PlanOutput {
    plan_id: String,
    account: AccountSummaryOutput,
    name: String,
    monthly_usd: f64,
    period_start: String,
    period_end: Option<String>,
}

/// One billing period. `period_end` is exclusive; `leverage` is list cost over the plan's
/// price, absent for a free plan.
#[derive(Debug, Serialize, JsonSchema)]
struct LeveragePeriodOutput {
    period_start: String,
    period_end: String,
    complete: bool,
    requests: i64,
    total_tokens: i64,
    list_cost_usd: f64,
    leverage: Option<f64>,
    effective_usd_per_mtok: Option<f64>,
}

struct UsageTools {
    state: Arc<AppState>,
}

#[Tools]
impl UsageTools {
    /// List the gateways this service meters.
    async fn list_sources(&self) -> Result<StructuredContent<SourcesOutput>, String> {
        let sources = Source::list(&self.state.database)
            .await
            .map_err(|error| refuse("MCP source listing failed", &error))?;

        Ok(StructuredContent(SourcesOutput {
            sources: sources
                .into_iter()
                .map(|source| SourceOutput {
                    source_id: source.id.encode(),
                    name: source.name,
                    kind: source.kind.stored().to_owned(),
                    base_url: source.base_url,
                    enabled: source.enabled,
                    created_at: source.created_at.to_string(),
                    last_event_at: source.last_event_at.map(|at| at.0.to_string()),
                    event_count: source.event_count,
                })
                .collect(),
        }))
    }

    /// List the upstream credentials requests were charged to, most recently used first.
    async fn list_accounts(&self) -> Result<StructuredContent<AccountsOutput>, String> {
        let accounts = Account::list(&self.state.database)
            .await
            .map_err(|error| refuse("MCP account listing failed", &error))?;

        Ok(StructuredContent(AccountsOutput {
            accounts: accounts.into_iter().map(AccountOutput::from).collect(),
        }))
    }

    /// Summarize every metered request in a window: totals, cost, cache savings, the rate
    /// per local day and the busiest days. Both bounds are RFC 3339 and optional; `to` is
    /// exclusive and defaults to now, and `from` absent starts at the first request.
    /// `utc_offset_minutes` places day boundaries in local time.
    async fn usage_summary(
        &self,
        from: Option<String>,
        to: Option<String>,
        utc_offset_minutes: Option<i32>,
    ) -> Result<StructuredContent<SummaryOutput>, String> {
        let offset = utc_offset_minutes
            .unwrap_or(0)
            .checked_mul(60)
            .and_then(|seconds| Offset::from_seconds(seconds).ok())
            .ok_or_else(|| "utc_offset_minutes is outside the supported range".to_owned())?;
        let filter = AnalyticsFilter::window(
            instant(from.as_deref(), "from")?,
            instant(to.as_deref(), "to")?,
            offset,
        );
        let summary = Summary::load(&self.state.database, &filter)
            .await
            .map_err(|error| match error {
                AnalyticsError::Database(error) => refuse("MCP summary failed", &error),
                refused => refused.to_string(),
            })?;

        Ok(StructuredContent(summary.into()))
    }

    /// The newest metered requests, most recent first, each with the account it was
    /// charged to.
    async fn recent_events(
        &self,
        limit: Option<i64>,
        failed_only: Option<bool>,
    ) -> Result<StructuredContent<EventsOutput>, String> {
        let filter = UsageFilter {
            failed: failed_only.filter(|only| *only),
            ..UsageFilter::default()
        };
        let limit = limit.unwrap_or(DEFAULT_EVENTS).clamp(1, MAX_EVENTS);
        let events = UsageEvent::page(&self.state.database, &filter, limit, None)
            .await
            .map_err(|error| refuse("MCP event listing failed", &error))?;

        Ok(StructuredContent(EventsOutput {
            events: events
                .into_iter()
                .map(|event| EventOutput {
                    event_id: event.id.encode(),
                    source_id: event.source_id.encode(),
                    occurred_at: event.occurred_at.to_string(),
                    provider: event.provider,
                    model: event.model,
                    account: event.account.into(),
                    harness: event.harness,
                    input_tokens: event.input_tokens,
                    output_tokens: event.output_tokens,
                    total_tokens: event.total_tokens,
                    status_code: event.status_code,
                    failed: event.failed,
                    error_message: event.error_message,
                })
                .collect(),
        }))
    }

    /// The last known health and quota of every CLIProxy credential, optionally of one
    /// source, from this service's database. Reaches neither CLIProxy nor any provider.
    async fn quota_status(
        &self,
        source_id: Option<String>,
    ) -> Result<StructuredContent<QuotaStatusOutput>, String> {
        let source_id = source_id
            .as_deref()
            .map(str::parse::<Id<Source>>)
            .transpose()
            .map_err(|_| "source_id is not an id".to_owned())?;
        let accounts = QuotaAccount::list(&self.state.database, source_id)
            .await
            .map_err(|error| refuse("MCP quota listing failed", &error))?;

        Ok(StructuredContent(QuotaStatusOutput {
            accounts: accounts.into_iter().map(QuotaAccountOutput::from).collect(),
        }))
    }

    /// What each subscription plan was worth: for its latest billing periods, newest
    /// first, the list cost of its account's requests against the plan's monthly price.
    async fn leverage(&self) -> Result<StructuredContent<LeverageOutput>, String> {
        let plans = PlanLeverage::list(
            &self.state.database,
            DEFAULT_LEVERAGE_PERIODS,
            Timestamp::now(),
        )
        .await
        .map_err(|error| refuse("MCP leverage failed", &error))?;

        Ok(StructuredContent(LeverageOutput {
            plans: plans.into_iter().map(PlanLeverageOutput::from).collect(),
        }))
    }
}

/// poem-mcpserver 0.3.1 picks the response framing from the first entry of `Accept` alone,
/// and its JSON arm answers even a single request with a one-element array. A client that
/// reads one response object, as the Streamable HTTP transport is specified to, rejects
/// that as malformed. The crate's event-stream arm is correct, so a client that offered
/// `text/event-stream` anywhere is given it by naming it first. A client that never
/// offered it keeps the framing it asked for.
async fn prefer_event_stream(mut request: Request) -> poem::Result<Request> {
    if request
        .header(header::ACCEPT)
        .is_some_and(|accept| accept.contains(EVENT_STREAM))
    {
        request.headers_mut().insert(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream, application/json"),
        );
    }

    Ok(request)
}

fn instant(value: Option<&str>, name: &'static str) -> Result<Option<Timestamp>, String> {
    value
        .map(str::parse::<Timestamp>)
        .transpose()
        .map_err(|_| format!("{name} must be an RFC 3339 timestamp"))
}

fn refuse(message: &'static str, error: &DatabaseError) -> String {
    tracing::error!(%error, message);
    "the database refused the request".to_owned()
}

fn server(state: Arc<AppState>) -> impl IntoEndpoint {
    streamable_http::endpoint(move |_: &Request| {
        McpServer::new()
            .tools(UsageTools {
                state: Arc::clone(&state),
            })
            .with_server_info("metered-usage", env!("CARGO_PKG_VERSION"))
    })
}

pub fn endpoint(state: Arc<AppState>) -> impl IntoEndpoint {
    server(state).into_endpoint().before(prefer_event_stream)
}

#[cfg(test)]
mod tests {
    use poem::http::{Method, StatusCode, Uri};
    use poem::{Endpoint, Response};

    use super::*;

    async fn state() -> Arc<AppState> {
        let database = Database::open("sqlite::memory:", 0)
            .await
            .expect("an in-memory database");

        let token = "a token".to_owned().try_into().expect("a token");

        Arc::new(
            AppState::new(database, token, crate::config::Config::default())
                .expect("an HTTP client"),
        )
    }

    async fn probe() -> impl Endpoint<Output = Response> {
        endpoint(state().await).into_endpoint().map_to_response()
    }

    async fn session_of(endpoint: &impl Endpoint<Output = Response>) -> String {
        let initialize = endpoint
            .call(post(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}"#,
                None,
            ))
            .await
            .expect("an initialize response");
        assert_eq!(initialize.status(), StatusCode::OK);

        initialize
            .headers()
            .get("Mcp-Session-Id")
            .expect("a session")
            .to_str()
            .expect("a readable session")
            .to_owned()
    }

    /// The `Accept` order every Streamable HTTP client sends, and the one that selects the
    /// arm of poem-mcpserver that answers a single request with an array.
    fn post(body: &'static str, session: Option<&str>) -> Request {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(Uri::from_static("/"))
            .content_type("application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        if let Some(session) = session {
            builder = builder.header("Mcp-Session-Id", session);
        }

        builder.body(body)
    }

    #[tokio::test]
    async fn a_call_answers_one_response_and_not_an_array_of_one() {
        let endpoint = probe().await;
        let session = session_of(&endpoint).await;

        let listed = endpoint
            .call(post(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
                Some(&session),
            ))
            .await
            .unwrap();
        assert!(
            listed
                .content_type()
                .is_some_and(|content_type| content_type.starts_with(EVENT_STREAM)),
            "framing must be the arm that answers one response per event"
        );

        let body = listed.into_body().into_string().await.expect("a body");
        let frames = body
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .collect::<Vec<_>>();
        assert_eq!(frames.len(), 1, "one request, one response: {body}");

        let response: serde_json::Value =
            serde_json::from_str(frames[0].trim()).expect("a JSON-RPC response");
        assert!(response.is_object(), "not an array of one: {body}");
        assert_eq!(response["id"], 2);
    }

    /// Why [`prefer_event_stream`] exists. When this fails, poem-mcpserver has learned to
    /// answer a single request with a single object and the nudge can be deleted.
    #[tokio::test]
    async fn the_json_arm_still_answers_a_single_request_with_an_array() {
        let endpoint = server(state().await).into_endpoint().map_to_response();
        let session = session_of(&endpoint).await;

        let listed = endpoint
            .call(post(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
                Some(&session),
            ))
            .await
            .unwrap();
        let body = listed.into_body().into_string().await.expect("a body");
        let response: serde_json::Value = serde_json::from_str(&body).expect("a JSON body");

        assert!(response.is_array(), "the crate was fixed upstream: {body}");
    }
}
