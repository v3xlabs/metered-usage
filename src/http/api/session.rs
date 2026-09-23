use std::sync::Arc;

use poem::Request;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object, OpenApi};

use crate::app::AppState;
use crate::http::api::Error;
use crate::http::auth::Token;

pub struct SessionApi {
    pub state: Arc<AppState>,
}

#[OpenApi]
impl SessionApi {
    /// Answers whether the request carries the access token.
    #[oai(path = "/session", method = "get", operation_id = "get_session")]
    async fn get_session(&self, request: &Request) -> SessionResponse {
        if self.state.token.admits(request) {
            SessionResponse::Authenticated
        } else {
            SessionResponse::Unauthorized(Json(refused()))
        }
    }

    /// Trades the access token for a session cookie.
    #[oai(path = "/session", method = "post", operation_id = "create_session")]
    async fn create_session(&self, input: Json<SessionInput>) -> CreateSessionResponse {
        if self.state.token.matches(&input.0.token) {
            CreateSessionResponse::Created(self.state.token.session_cookie())
        } else {
            CreateSessionResponse::Unauthorized(Json(refused()))
        }
    }

    #[oai(path = "/session", method = "delete", operation_id = "delete_session")]
    async fn delete_session(&self) -> DeleteSessionResponse {
        DeleteSessionResponse::Deleted(Token::cleared_cookie())
    }
}

#[derive(Debug, Object)]
struct SessionInput {
    token: String,
}

#[derive(ApiResponse)]
enum SessionResponse {
    #[oai(status = 204)]
    Authenticated,
    #[oai(status = 401)]
    Unauthorized(Json<Error>),
}

#[derive(ApiResponse)]
enum CreateSessionResponse {
    #[oai(status = 204)]
    Created(#[oai(header = "Set-Cookie")] String),
    #[oai(status = 401)]
    Unauthorized(Json<Error>),
}

#[derive(ApiResponse)]
enum DeleteSessionResponse {
    #[oai(status = 204)]
    Deleted(#[oai(header = "Set-Cookie")] String),
}

fn refused() -> Error {
    Error {
        message: "the access token is not valid".to_owned(),
    }
}
