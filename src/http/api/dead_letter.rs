use std::sync::Arc;

use poem_openapi::param::Query;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::prelude::*;
use crate::usage::dead_letter::DeadLetter;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 500;

pub struct DeadLetterApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl DeadLetterApi {
    /// Records a gateway sent that could not be read, newest first. Each is kept for 14
    /// days, with every field that could carry a credential removed.
    #[oai(
        path = "/dead-letters",
        method = "get",
        operation_id = "list_dead_letters"
    )]
    async fn list_dead_letters(
        &self,
        source_id: Query<Option<String>>,
        limit: Query<Option<i64>>,
    ) -> DeadLetterListResponse {
        let source_id = match source_id.0.as_deref().map(str::parse::<Id<Source>>) {
            None => None,
            Some(Ok(source_id)) => Some(source_id),
            Some(Err(_)) => {
                return DeadLetterListResponse::Invalid(Json(Error {
                    message: "source_id must be a source id".to_owned(),
                }));
            }
        };
        let limit = limit.0.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        match DeadLetter::list(&self.state.database, source_id, limit).await {
            Ok(dead_letters) => DeadLetterListResponse::Found(Json(DeadLetterList {
                dead_letters: dead_letters
                    .into_iter()
                    .map(DeadLetterOutput::from)
                    .collect(),
            })),
            Err(error) => {
                tracing::error!(%error, "dead letter listing failed");
                DeadLetterListResponse::Failed(Json(Error {
                    message: "the database refused the request".to_owned(),
                }))
            }
        }
    }
}

#[derive(Debug, Object)]
#[oai(rename = "DeadLetter")]
struct DeadLetterOutput {
    dead_letter_id: String,
    source_id: String,
    received_at: String,
    error: String,
    /// The record as JSON text.
    payload: String,
}

impl From<DeadLetter> for DeadLetterOutput {
    fn from(dead_letter: DeadLetter) -> Self {
        Self {
            dead_letter_id: dead_letter.id.encode(),
            source_id: dead_letter.source_id.encode(),
            received_at: dead_letter.received_at.to_string(),
            error: dead_letter.error,
            payload: dead_letter.payload,
        }
    }
}

#[derive(Debug, Object)]
struct DeadLetterList {
    dead_letters: Vec<DeadLetterOutput>,
}

#[derive(ApiResponse)]
enum DeadLetterListResponse {
    #[oai(status = 200)]
    Found(Json<DeadLetterList>),
    #[oai(status = 400)]
    Invalid(Json<Error>),
    #[oai(status = 500)]
    Failed(Json<Error>),
}
