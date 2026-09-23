use std::sync::Arc;

use jiff::Timestamp;
use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::plan::PlanOutput;
use crate::plan::leverage::{BillingPeriod, PlanLeverage};

pub const DEFAULT_PERIODS: usize = 6;
pub const MAX_PERIODS: usize = 24;

pub struct LeverageApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl LeverageApi {
    /// Every plan's latest billing periods, newest first, each with the list cost of the
    /// plan's account in it against the plan's price.
    #[oai(path = "/leverage", method = "get", operation_id = "leverage")]
    async fn leverage(&self, periods: Query<Option<u32>>) -> LeverageResponse {
        let periods = periods
            .0
            .and_then(|periods| usize::try_from(periods).ok())
            .unwrap_or(DEFAULT_PERIODS)
            .clamp(1, MAX_PERIODS);
        match PlanLeverage::list(&self.state.database, periods, Timestamp::now()).await {
            Ok(plans) => LeverageResponse::Found(Json(Leverage {
                plans: plans.into_iter().map(PlanLeverageOutput::from).collect(),
            })),
            Err(error) => {
                tracing::error!(%error, "leverage failed");
                LeverageResponse::Failed(Json(Error {
                    message: "the database refused the request".to_owned(),
                }))
            }
        }
    }
}

#[derive(Debug, Object)]
struct Leverage {
    plans: Vec<PlanLeverageOutput>,
}

#[derive(Debug, Object)]
#[oai(rename = "PlanLeverage")]
struct PlanLeverageOutput {
    plan: PlanOutput,
    /// Newest first.
    periods: Vec<LeveragePeriod>,
}

impl From<PlanLeverage> for PlanLeverageOutput {
    fn from(leverage: PlanLeverage) -> Self {
        Self {
            plan: leverage.plan.into(),
            periods: leverage
                .periods
                .into_iter()
                .map(LeveragePeriod::from)
                .collect(),
        }
    }
}

/// `period_end` is exclusive. `leverage` is list cost over the plan's price and absent for
/// a free plan; `effective_usd_per_mtok` is absent when no token was used.
#[derive(Debug, Object)]
#[oai(skip_serializing_if_is_none)]
struct LeveragePeriod {
    period_start: String,
    period_end: String,
    complete: bool,
    requests: i64,
    total_tokens: i64,
    list_cost_usd: f64,
    leverage: Option<f64>,
    effective_usd_per_mtok: Option<f64>,
}

impl From<BillingPeriod> for LeveragePeriod {
    fn from(period: BillingPeriod) -> Self {
        Self {
            period_start: period.start.to_string(),
            period_end: period.end.to_string(),
            complete: period.complete,
            requests: period.requests,
            total_tokens: period.total_tokens,
            list_cost_usd: period.list_cost_usd,
            leverage: period.leverage,
            effective_usd_per_mtok: period.effective_usd_per_mtok,
        }
    }
}

#[derive(ApiResponse)]
enum LeverageResponse {
    #[oai(status = 200)]
    Found(Json<Leverage>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}
