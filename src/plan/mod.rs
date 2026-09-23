//! A subscription paid for an account. Its price is set against the list cost of each
//! billing period, never spread over requests.

pub mod leverage;

use jiff::Timestamp;
use sqlx::{AssertSqlSafe, FromRow};

use crate::account::{Account, AccountSummary};
use crate::database::codec::StoredTimestamp;
use crate::database::{Database, DatabaseError};
use crate::id::Id;

const SELECT: &str = "SELECT plan.plan_id, plan.name, plan.monthly_usd, plan.period_start, \
     plan.period_end, account.account_id, account.source_id, source.name AS source_name, \
     account.provider, account.auth_kind, account.label, account.display_name \
     FROM plan \
     JOIN account ON account.account_id = plan.account_id \
     JOIN source ON source.source_id = account.source_id";

#[derive(Debug, Clone, FromRow)]
pub struct Plan {
    #[sqlx(rename = "plan_id")]
    pub id: Id<Plan>,
    #[sqlx(flatten)]
    pub account: AccountSummary,
    pub name: String,
    pub monthly_usd: f64,
    #[sqlx(try_from = "StoredTimestamp")]
    pub period_start: Timestamp,
    pub period_end: Option<StoredTimestamp>,
}

impl Plan {
    pub async fn list(database: &Database) -> Result<Vec<Self>, DatabaseError> {
        Ok(sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} ORDER BY plan.period_start, plan.plan_id"
        )))
        .fetch_all(&database.pool)
        .await?)
    }

    pub async fn load(database: &Database, id: Id<Plan>) -> Result<Option<Self>, DatabaseError> {
        Ok(
            sqlx::query_as::<_, Self>(AssertSqlSafe(format!("{SELECT} WHERE plan.plan_id = ?")))
                .bind(id)
                .fetch_optional(&database.pool)
                .await?,
        )
    }

    pub async fn create(database: &Database, plan: &NewPlan) -> Result<Self, PlanError> {
        plan.check()?;
        let id = database.ids.next::<Self>();
        sqlx::query(
            "INSERT INTO plan (plan_id, account_id, name, monthly_usd, period_start, period_end) \
             SELECT ?, account_id, ?, ?, ?, ? FROM account WHERE account_id = ?",
        )
        .bind(id)
        .bind(plan.name.trim())
        .bind(plan.monthly_usd)
        .bind(StoredTimestamp::from(plan.period_start))
        .bind(plan.period_end.map(StoredTimestamp::from))
        .bind(plan.account_id)
        .execute(&database.pool)
        .await
        .map_err(DatabaseError::from)?;

        Self::load(database, id)
            .await?
            .ok_or(PlanError::UnknownAccount)
    }

    /// Answers nothing when there is no such plan.
    pub async fn update(
        database: &Database,
        id: Id<Plan>,
        change: PlanChange,
    ) -> Result<Option<Self>, PlanError> {
        let Some(plan) = Self::load(database, id).await? else {
            return Ok(None);
        };
        let plan = NewPlan {
            account_id: change.account_id.unwrap_or(plan.account.id),
            name: change.name.unwrap_or(plan.name),
            monthly_usd: change.monthly_usd.unwrap_or(plan.monthly_usd),
            period_start: change.period_start.unwrap_or(plan.period_start),
            period_end: change
                .period_end
                .unwrap_or_else(|| plan.period_end.map(Timestamp::from)),
        };
        plan.check()?;
        let updated = sqlx::query(
            "UPDATE plan SET account_id = account.account_id, name = ?, monthly_usd = ?, \
             period_start = ?, period_end = ? \
             FROM account WHERE account.account_id = ? AND plan.plan_id = ?",
        )
        .bind(plan.name.trim())
        .bind(plan.monthly_usd)
        .bind(StoredTimestamp::from(plan.period_start))
        .bind(plan.period_end.map(StoredTimestamp::from))
        .bind(plan.account_id)
        .bind(id)
        .execute(&database.pool)
        .await
        .map_err(DatabaseError::from)?;
        if updated.rows_affected() == 0 {
            return Err(PlanError::UnknownAccount);
        }

        Ok(Self::load(database, id).await?)
    }

    /// Answers whether there was a plan to delete.
    pub async fn delete(database: &Database, id: Id<Plan>) -> Result<bool, DatabaseError> {
        let deleted = sqlx::query("DELETE FROM plan WHERE plan_id = ?")
            .bind(id)
            .execute(&database.pool)
            .await?;

        Ok(deleted.rows_affected() > 0)
    }
}

#[derive(Debug)]
pub struct NewPlan {
    pub account_id: Id<Account>,
    pub name: String,
    pub monthly_usd: f64,
    pub period_start: Timestamp,
    pub period_end: Option<Timestamp>,
}

impl NewPlan {
    fn check(&self) -> Result<(), PlanError> {
        if self.name.trim().is_empty() {
            return Err(PlanError::BlankName);
        }
        if !self.monthly_usd.is_finite() || self.monthly_usd < 0.0 {
            return Err(PlanError::Price);
        }
        if self.period_end.is_some_and(|end| end <= self.period_start) {
            return Err(PlanError::EmptyPeriod);
        }

        Ok(())
    }
}

/// Every field absent keeps its value. `period_end` is `Some(None)` to clear it.
#[derive(Debug, Default)]
pub struct PlanChange {
    pub account_id: Option<Id<Account>>,
    pub name: Option<String>,
    pub monthly_usd: Option<f64>,
    pub period_start: Option<Timestamp>,
    pub period_end: Option<Option<Timestamp>>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("name must not be blank")]
    BlankName,
    #[error("monthly_usd must be a number no less than 0")]
    Price,
    #[error("period_end must be after period_start")]
    EmptyPeriod,
    #[error("{0} must be an RFC 3339 timestamp")]
    Timestamp(&'static str),
    #[error("account was not found")]
    UnknownAccount,
    #[error(transparent)]
    Database(#[from] DatabaseError),
}
