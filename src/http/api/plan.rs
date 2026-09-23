use std::sync::Arc;

use jiff::Timestamp;
use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::types::MaybeUndefined;
use poem_openapi::{ApiResponse, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::api::account::AccountSummaryOutput;
use crate::plan::{NewPlan, Plan, PlanChange, PlanError};
use crate::prelude::*;

pub struct PlanApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl PlanApi {
    #[oai(path = "/plans", method = "get", operation_id = "list_plans")]
    async fn list_plans(&self) -> PlanListResponse {
        match Plan::list(&self.state.database).await {
            Ok(plans) => PlanListResponse::Found(Json(PlanList {
                plans: plans.into_iter().map(PlanOutput::from).collect(),
            })),
            Err(error) => PlanListResponse::Failed(Json(failure("list_plans", &error))),
        }
    }

    #[oai(path = "/plans", method = "post", operation_id = "create_plan")]
    async fn create_plan(&self, input: Json<PlanInput>) -> PlanResponse {
        let plan = match NewPlan::try_from(input.0) {
            Ok(plan) => plan,
            Err(error) => return Err::<Plan, _>(error).into(),
        };
        Plan::create(&self.state.database, &plan).await.into()
    }

    /// Absent fields keep their value; a null `period_end` makes the plan open ended.
    #[oai(
        path = "/plans/:plan_id",
        method = "patch",
        operation_id = "update_plan"
    )]
    async fn update_plan(&self, plan_id: Path<String>, input: Json<PlanUpdate>) -> PlanResponse {
        let Ok(plan_id) = plan_id.0.parse::<Id<Plan>>() else {
            return PlanResponse::Missing(Json(missing()));
        };
        let change = match PlanChange::try_from(input.0) {
            Ok(change) => change,
            Err(error) => return Err::<Plan, _>(error).into(),
        };
        match Plan::update(&self.state.database, plan_id, change).await {
            Ok(Some(plan)) => Ok::<_, PlanError>(plan).into(),
            Ok(None) => PlanResponse::Missing(Json(missing())),
            Err(error) => Err::<Plan, _>(error).into(),
        }
    }

    #[oai(
        path = "/plans/:plan_id",
        method = "delete",
        operation_id = "delete_plan"
    )]
    async fn delete_plan(&self, plan_id: Path<String>) -> DeleteResponse {
        let Ok(plan_id) = plan_id.0.parse::<Id<Plan>>() else {
            return DeleteResponse::Missing(Json(missing()));
        };
        match Plan::delete(&self.state.database, plan_id).await {
            Ok(true) => DeleteResponse::Deleted,
            Ok(false) => DeleteResponse::Missing(Json(missing())),
            Err(error) => DeleteResponse::Failed(Json(failure("delete_plan", &error))),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Plan", skip_serializing_if_is_none)]
pub struct PlanOutput {
    plan_id: String,
    account: AccountSummaryOutput,
    name: String,
    monthly_usd: f64,
    period_start: String,
    period_end: Option<String>,
}

impl From<Plan> for PlanOutput {
    fn from(plan: Plan) -> Self {
        Self {
            plan_id: plan.id.encode(),
            account: plan.account.into(),
            name: plan.name,
            monthly_usd: plan.monthly_usd,
            period_start: plan.period_start.to_string(),
            period_end: plan.period_end.map(|end| end.0.to_string()),
        }
    }
}

#[derive(Debug, Object)]
struct PlanList {
    plans: Vec<PlanOutput>,
}

/// Billing periods are calendar months from `period_start`'s day of month.
#[derive(Debug, Object)]
struct PlanInput {
    account_id: String,
    name: String,
    monthly_usd: f64,
    period_start: String,
    period_end: Option<String>,
}

impl TryFrom<PlanInput> for NewPlan {
    type Error = PlanError;

    fn try_from(input: PlanInput) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: account(&input.account_id)?,
            name: input.name,
            monthly_usd: input.monthly_usd,
            period_start: instant(&input.period_start, "period_start")?,
            period_end: input
                .period_end
                .as_deref()
                .map(|end| instant(end, "period_end"))
                .transpose()?,
        })
    }
}

#[derive(Debug, Object)]
struct PlanUpdate {
    account_id: Option<String>,
    name: Option<String>,
    monthly_usd: Option<f64>,
    period_start: Option<String>,
    #[oai(nullable)]
    period_end: MaybeUndefined<String>,
}

impl TryFrom<PlanUpdate> for PlanChange {
    type Error = PlanError;

    fn try_from(input: PlanUpdate) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: input.account_id.as_deref().map(account).transpose()?,
            name: input.name,
            monthly_usd: input.monthly_usd,
            period_start: input
                .period_start
                .as_deref()
                .map(|start| instant(start, "period_start"))
                .transpose()?,
            period_end: match input.period_end {
                MaybeUndefined::Undefined => None,
                MaybeUndefined::Null => Some(None),
                MaybeUndefined::Value(end) => Some(Some(instant(&end, "period_end")?)),
            },
        })
    }
}

#[derive(ApiResponse)]
enum PlanListResponse {
    #[oai(status = 200)]
    Found(Json<PlanList>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum PlanResponse {
    #[oai(status = 200)]
    Found(Json<Box<PlanOutput>>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

impl From<Result<Plan, PlanError>> for PlanResponse {
    fn from(result: Result<Plan, PlanError>) -> Self {
        match result {
            Ok(plan) => Self::Found(Json(Box::new(plan.into()))),
            Err(PlanError::UnknownAccount) => Self::Missing(Json(Error {
                message: PlanError::UnknownAccount.to_string(),
            })),
            Err(PlanError::Database(error)) => Self::Failed(Json(failure("plan", &error))),
            Err(error) => Self::Invalid(Json(Error {
                message: error.to_string(),
            })),
        }
    }
}

#[derive(ApiResponse)]
enum DeleteResponse {
    #[oai(status = 204)]
    Deleted,
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn account(id: &str) -> Result<Id<Account>, PlanError> {
    id.parse().map_err(|_| PlanError::UnknownAccount)
}

fn instant(value: &str, name: &'static str) -> Result<Timestamp, PlanError> {
    value.parse().map_err(|_| PlanError::Timestamp(name))
}

fn missing() -> Error {
    Error {
        message: "plan was not found".to_owned(),
    }
}

fn failure(operation: &'static str, error: &DatabaseError) -> Error {
    tracing::error!(%operation, %error, "plan operation failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
