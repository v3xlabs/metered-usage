use std::sync::Arc;

use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};

use crate::app::AppState;
use crate::http::VERSION;

pub struct HealthApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl HealthApi {
    #[oai(path = "/health", method = "get", operation_id = "health")]
    async fn health(&self) -> Json<Health> {
        let reachable = sqlx::query("SELECT 1")
            .fetch_one(&self.state.database.pool)
            .await
            .is_ok();

        Json(Health {
            status: if reachable { "ok" } else { "degraded" }.to_owned(),
            version: VERSION.to_owned(),
        })
    }
}

#[derive(Debug, Object)]
struct Health {
    status: String,
    version: String,
}
