//! A subscription paid for an account. Its price is set against the list cost of each
//! billing period, never spread over requests.
//!
//! Every write that adds, edits or removes a plan records a `plan.changed` in its own
//! transaction. An edit that leaves the plan as it was, is refused, or names no plan
//! records none.

pub mod leverage;

use jiff::Timestamp;
use sqlx::{AssertSqlSafe, FromRow, SqliteConnection};

use crate::account::{Account, AccountSummary};
use crate::change::Subject;
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
    /// The credential that replaced the plan's account, when nothing else could have. The
    /// plan stays on its account until an explicit merge; the replacement's requests count
    /// towards its leverage meanwhile.
    #[sqlx(skip)]
    pub replaced_by_account_id: Option<Id<Account>>,
}

impl Plan {
    pub async fn list(database: &Database) -> Result<Vec<Self>, DatabaseError> {
        let mut transaction = database.pool.begin().await?;
        let mut plans = sqlx::query_as::<_, Self>(AssertSqlSafe(format!(
            "{SELECT} ORDER BY plan.period_start, plan.plan_id"
        )))
        .fetch_all(&mut *transaction)
        .await?;
        let replacements = Account::replacements(&mut transaction).await?;
        transaction.commit().await?;
        for plan in &mut plans {
            plan.replaced_by_account_id = replacements.get(&plan.account.id).copied();
        }
        Ok(plans)
    }

    pub async fn load(database: &Database, id: Id<Plan>) -> Result<Option<Self>, DatabaseError> {
        let mut connection = database.pool.acquire().await?;
        Self::read(&mut connection, id).await
    }

    pub async fn create(database: &Database, plan: &NewPlan) -> Result<Self, PlanError> {
        plan.check()?;
        let id = database.ids.next::<Self>();
        let mut write = database.write().await?;
        let linked = Account::replacements(&mut write).await?;
        let inserted = sqlx::query(
            "INSERT INTO plan (plan_id, account_id, name, monthly_usd, period_start, period_end) \
             SELECT ?, account_id, ?, ?, ?, ? FROM account WHERE account_id = ?",
        )
        .bind(id)
        .bind(plan.name.trim())
        .bind(plan.monthly_usd)
        .bind(StoredTimestamp::from(plan.period_start))
        .bind(plan.period_end.map(StoredTimestamp::from))
        .bind(plan.account_id)
        .execute(&mut *write)
        .await
        .map_err(DatabaseError::from)?;
        if inserted.rows_affected() == 0 {
            return Err(PlanError::UnknownAccount);
        }
        let created = Self::read(&mut write, id)
            .await?
            .ok_or(PlanError::UnknownAccount)?;
        Account::record_link_changes(&mut write, &linked, [Subject::Plan(id)]).await?;
        write.commit().await?;

        Ok(created)
    }

    /// Answers nothing when there is no such plan. The plan is read and written in one
    /// transaction, so an edit to one field never undoes a concurrent edit to another.
    pub async fn update(
        database: &Database,
        id: Id<Plan>,
        change: PlanChange,
    ) -> Result<Option<Self>, PlanError> {
        let mut write = database.write().await?;
        let Some(current) = Self::read(&mut write, id).await? else {
            return Ok(None);
        };
        // A stored plan passed the checks, so one that reads the same needs none.
        if !change.alters(&current) {
            return Ok(Some(current));
        }
        let plan = NewPlan {
            account_id: change.account_id.unwrap_or(current.account.id),
            name: change.name.unwrap_or(current.name),
            monthly_usd: change.monthly_usd.unwrap_or(current.monthly_usd),
            period_start: change.period_start.unwrap_or(current.period_start),
            period_end: change
                .period_end
                .unwrap_or_else(|| current.period_end.map(Timestamp::from)),
        };
        plan.check()?;
        let linked = Account::replacements(&mut write).await?;
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
        .execute(&mut *write)
        .await
        .map_err(DatabaseError::from)?;
        if updated.rows_affected() == 0 {
            return Err(PlanError::UnknownAccount);
        }
        let edited = Self::read(&mut write, id).await?;
        Account::record_link_changes(&mut write, &linked, [Subject::Plan(id)]).await?;
        write.commit().await?;

        Ok(edited)
    }

    /// Answers whether there was a plan to delete.
    pub async fn delete(database: &Database, id: Id<Plan>) -> Result<bool, DatabaseError> {
        let mut write = database.write().await?;
        let linked = Account::replacements(&mut write).await?;
        let deleted = sqlx::query("DELETE FROM plan WHERE plan_id = ?")
            .bind(id)
            .execute(&mut *write)
            .await?;
        if deleted.rows_affected() == 0 {
            return Ok(false);
        }
        Account::record_link_changes(&mut write, &linked, [Subject::Plan(id)]).await?;
        write.commit().await?;

        Ok(true)
    }

    async fn read(
        connection: &mut SqliteConnection,
        id: Id<Plan>,
    ) -> Result<Option<Self>, DatabaseError> {
        let mut plan =
            sqlx::query_as::<_, Self>(AssertSqlSafe(format!("{SELECT} WHERE plan.plan_id = ?")))
                .bind(id)
                .fetch_optional(&mut *connection)
                .await?;
        if let Some(plan) = plan.as_mut() {
            let replacements = Account::replacements(connection).await?;
            plan.replaced_by_account_id = replacements.get(&plan.account.id).copied();
        }
        Ok(plan)
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

impl PlanChange {
    /// Whether the plan would read differently after the change. A name is compared as
    /// it is stored, trimmed. A price is compared bit for bit, as its JSON is: `-0.0` is
    /// not `0.0`, and `NaN` always differs, so it reaches the check that refuses it.
    fn alters(&self, plan: &Plan) -> bool {
        self.account_id.is_some_and(|id| id != plan.account.id)
            || self
                .name
                .as_deref()
                .is_some_and(|name| name.trim() != plan.name)
            || self
                .monthly_usd
                .is_some_and(|usd| usd.to_bits() != plan.monthly_usd.to_bits())
            || self
                .period_start
                .is_some_and(|start| start != plan.period_start)
            || self
                .period_end
                .is_some_and(|end| end != plan.period_end.map(Timestamp::from))
    }
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
