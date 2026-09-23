//! One shared token guards everything that reads or writes usage. A client presents it as
//! a bearer token, or as the session cookie a browser was given for it.

use std::sync::Arc;

use poem::http::{Method, StatusCode, header};
use poem::{Endpoint, IntoResponse, Middleware, Request, Response};

pub const SESSION_COOKIE: &str = "metered_usage_session";
const COOKIE_ATTRIBUTES: &str = "HttpOnly; SameSite=Strict; Path=/";

/// Operations under [`crate::http::MOUNT`] a client reaches before it has a credential. `GET /session`
/// is here because it answers the question of whether the caller has one.
const OPEN: [(Method, &str); 3] = [
    (Method::GET, "/health"),
    (Method::POST, "/session"),
    (Method::GET, "/session"),
];

#[derive(Clone)]
pub struct Token(Arc<str>);

impl Token {
    /// Takes as long for a wrong guess sharing a prefix as for one sharing nothing, so the
    /// token cannot be recovered a character at a time. Only its length can be learned.
    #[must_use]
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let candidate = candidate.as_bytes();
        if expected.len() != candidate.len() {
            return false;
        }
        let difference = expected
            .iter()
            .zip(candidate)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            });

        std::hint::black_box(difference) == 0
    }

    #[must_use]
    pub fn admits(&self, request: &Request) -> bool {
        let bearer = request
            .header(header::AUTHORIZATION)
            .and_then(|value| value.strip_prefix("Bearer "));
        if bearer.is_some_and(|bearer| self.matches(bearer)) {
            return true;
        }

        request
            .headers()
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .any(|(name, value)| name == SESSION_COOKIE && self.matches(value))
    }

    /// The `Set-Cookie` value that makes a browser present this token.
    #[must_use]
    pub fn session_cookie(&self) -> String {
        format!("{SESSION_COOKIE}={}; {COOKIE_ATTRIBUTES}", self.0)
    }

    /// The `Set-Cookie` value that makes a browser forget it.
    #[must_use]
    pub fn cleared_cookie() -> String {
        format!("{SESSION_COOKIE}=; {COOKIE_ATTRIBUTES}; Max-Age=0")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the access token is empty")]
pub struct EmptyToken;

impl TryFrom<String> for Token {
    type Error = EmptyToken;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(EmptyToken);
        }

        Ok(Self(value.into()))
    }
}

/// Answers 401 to a request that did not present the token, unless it is one of the
/// [`OPEN`] operations.
pub struct RequireCredential {
    pub token: Token,
}

impl<E: Endpoint> Middleware<E> for RequireCredential {
    type Output = RequireCredentialEndpoint<E>;

    fn transform(&self, inner: E) -> Self::Output {
        RequireCredentialEndpoint {
            inner,
            token: self.token.clone(),
        }
    }
}

pub struct RequireCredentialEndpoint<E> {
    inner: E,
    token: Token,
}

impl<E: Endpoint> Endpoint for RequireCredentialEndpoint<E> {
    type Output = Response;

    async fn call(&self, request: Request) -> poem::Result<Self::Output> {
        if is_open(&request) || self.token.admits(&request) {
            return Ok(self.inner.call(request).await?.into_response());
        }

        Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .content_type("application/json")
            .body(serde_json::json!({ "message": "a credential is required" }).to_string()))
    }
}

/// The middleware is applied inside the nest at [`crate::http::MOUNT`], which has already taken the
/// prefix off the path it sees.
fn is_open(request: &Request) -> bool {
    let path = request.uri().path();

    OPEN.iter()
        .any(|(method, open)| request.method() == method && path == *open)
}
