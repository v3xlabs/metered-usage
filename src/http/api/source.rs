use std::sync::Arc;

use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::types::Any;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::prelude::*;
use crate::source::collector_state::CollectorState;
use crate::usage::cliproxy;
use crate::usage::ingest::{IngestReport, ingest};

pub struct SourceApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl SourceApi {
    #[oai(path = "/sources", method = "get", operation_id = "list_sources")]
    async fn list_sources(&self) -> SourceListResponse {
        match Source::list(&self.state.database).await {
            Ok(sources) => SourceListResponse::Found(Json(SourceList {
                sources: sources.into_iter().map(SourceOutput::from).collect(),
            })),
            Err(error) => SourceListResponse::Failed(Json(failure("list_sources", &error))),
        }
    }

    #[oai(
        path = "/sources/:source_id",
        method = "get",
        operation_id = "get_source"
    )]
    async fn get_source(&self, source_id: Path<String>) -> SourceResponse {
        let Ok(source_id) = source_id.0.parse::<Id<Source>>() else {
            return SourceResponse::Missing(Json(missing()));
        };
        match Source::load(&self.state.database, source_id).await {
            Ok(Some(source)) => SourceResponse::Found(Json(Box::new(source.into()))),
            Ok(None) => SourceResponse::Missing(Json(missing())),
            Err(error) => SourceResponse::Failed(Json(failure("get_source", &error))),
        }
    }

    /// Takes the gateway's own accounting records. A record this source already posted is
    /// counted as a duplicate, one this service cannot read is counted as rejected, and
    /// neither stops the rest of the batch.
    #[oai(
        path = "/sources/:source_id/events",
        method = "post",
        operation_id = "ingest_events"
    )]
    async fn ingest_events(
        &self,
        source_id: Path<String>,
        input: Json<IngestInput>,
    ) -> IngestResponse {
        let Ok(source_id) = source_id.0.parse::<Id<Source>>() else {
            return IngestResponse::Missing(Json(missing()));
        };
        match Source::load(&self.state.database, source_id).await {
            Ok(Some(_)) => {}
            Ok(None) => return IngestResponse::Missing(Json(missing())),
            Err(error) => return IngestResponse::Failed(Json(failure("ingest_events", &error))),
        }

        let records = input
            .0
            .records
            .into_iter()
            .map(|record| cliproxy::parse(record.0))
            .collect();
        match ingest(&self.state, source_id, records).await {
            Ok(report) => IngestResponse::Ingested(Json(report.into())),
            Err(error) => IngestResponse::Failed(Json(failure("ingest_events", &error))),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "Source", skip_serializing_if_is_none)]
struct SourceOutput {
    source_id: String,
    /// The config file's name for the source.
    key: String,
    name: String,
    kind: SourceKindOutput,
    base_url: String,
    enabled: bool,
    created_at: String,
    last_event_at: Option<String>,
    event_count: i64,
    collector: Option<CollectorStateOutput>,
}

impl From<Source> for SourceOutput {
    fn from(source: Source) -> Self {
        Self {
            source_id: source.id.encode(),
            key: source.key,
            name: source.name,
            kind: source.kind.into(),
            base_url: source.base_url,
            enabled: source.enabled,
            created_at: source.created_at.to_string(),
            last_event_at: source.last_event_at.map(|at| at.0.to_string()),
            event_count: source.event_count,
            collector: source.collector.map(CollectorStateOutput::from),
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "CollectorState", skip_serializing_if_is_none)]
struct CollectorStateOutput {
    connected_since: Option<String>,
    last_record_at: Option<String>,
    records_received: i64,
    consecutive_failures: i64,
    last_error: Option<String>,
    updated_at: String,
}

impl From<CollectorState> for CollectorStateOutput {
    fn from(state: CollectorState) -> Self {
        Self {
            connected_since: state.connected_since.map(|at| at.to_string()),
            last_record_at: state.last_record_at.map(|at| at.to_string()),
            records_received: state.records_received,
            consecutive_failures: state.consecutive_failures,
            last_error: state.last_error,
            updated_at: state.updated_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename = "SourceKind")]
enum SourceKindOutput {
    #[oai(rename = "cliproxy")]
    CliProxy,
    #[oai(rename = "litellm")]
    LiteLlm,
}

impl From<SourceKind> for SourceKindOutput {
    fn from(kind: SourceKind) -> Self {
        match kind {
            SourceKind::CliProxy => Self::CliProxy,
            SourceKind::LiteLlm => Self::LiteLlm,
        }
    }
}

#[derive(Debug, Object)]
struct SourceList {
    sources: Vec<SourceOutput>,
}

#[derive(Debug, Object)]
struct IngestInput {
    records: Vec<Any<serde_json::Value>>,
}

#[derive(Debug, Object)]
#[oai(rename = "IngestReport")]
struct IngestResult {
    accepted: i64,
    duplicates: i64,
    rejected: i64,
}

impl From<IngestReport> for IngestResult {
    fn from(report: IngestReport) -> Self {
        Self {
            accepted: report.accepted,
            duplicates: report.duplicates,
            rejected: report.rejected,
        }
    }
}

#[derive(ApiResponse)]
enum SourceListResponse {
    #[oai(status = 200)]
    Found(Json<SourceList>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum SourceResponse {
    #[oai(status = 200)]
    Found(Json<Box<SourceOutput>>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

#[derive(ApiResponse)]
enum IngestResponse {
    #[oai(status = 200)]
    Ingested(Json<IngestResult>),
    #[oai(status = 404)]
    Missing(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}

fn missing() -> Error {
    Error {
        message: "source was not found".to_owned(),
    }
}

fn failure(operation: &'static str, error: &DatabaseError) -> Error {
    tracing::error!(%operation, %error, "source operation failed");
    Error {
        message: "the database refused the request".to_owned(),
    }
}
