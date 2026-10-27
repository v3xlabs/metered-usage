//! One shared token guards everything that reads or writes usage. A client presents it as
//! a bearer token, or as the session cookie a browser was given for it. Two optional tokens
//! reach less: one every `GET`, for a dashboard, and one only what another service needs to
//! follow quota, resets, prices, accounts, usage summaries, plans, account health and
//! request arrivals, and to refresh quota.

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

/// Operations under [`crate::http::MOUNT`] the consumer token reaches, all `GET`: the
/// change feed, the lists its changes name, and the request and failure counts per account.
const CONSUMER_READS: [&str; 9] = [
    "/events",
    "/quota",
    "/quota/actions",
    "/prices",
    "/accounts",
    "/usage/summary",
    "/plans",
    "/leverage",
    "/analytics/health",
];

/// Operations under [`crate::http::MOUNT`] the consumer token reaches besides its reads, all
/// `POST`: a soft sync and a hard refresh. A hard refresh can spend quota, through xAI's paid
/// probe and Meta's key call, so a consumer sends one only for a person's click. The resets
/// spend a provider's grant or clear CLIProxy's cooldowns, so they stay with the token.
const CONSUMER_ACTIONS: [&str; 2] = ["/quota/sync", "/quota/refresh"];

/// The tokens the API admits, each to its own operations.
#[derive(Clone)]
pub struct Credentials {
    /// Admits every operation.
    pub token: Token,
    /// Admits every `GET`, for a dashboard such as Grafana.
    pub read_token: Option<Token>,
    /// Admits only the `GET`s of [`CONSUMER_READS`] and the `POST`s of [`CONSUMER_ACTIONS`],
    /// for a service such as Kettle Code that follows quota, prices, aggregate usage, plans,
    /// account health counts and request arrivals and refreshes quota, but cannot read
    /// request records.
    pub consumer_token: Option<Token>,
}

/// Which of the [`Credentials`] admitted a request.
#[derive(Clone, Copy)]
enum Role {
    Admin,
    Read,
    Consumer,
}

/// Credential role and transport, never a password-derived fingerprint. An app
/// label identifies a claimed client; this identifies the authority that admitted it.
#[derive(Clone, Copy)]
pub struct CredentialIdentity {
    pub credential_id: &'static str,
    pub transport: &'static str,
}

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

    /// How the request presents this token: `bearer` or `session`.
    #[must_use]
    pub fn presented(&self, request: &Request) -> Option<&'static str> {
        let bearer = request
            .header(header::AUTHORIZATION)
            .and_then(|value| value.strip_prefix("Bearer "));
        if bearer.is_some_and(|bearer| self.matches(bearer)) {
            return Some("bearer");
        }

        request
            .headers()
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .any(|(name, value)| name == SESSION_COOKIE && self.matches(value))
            .then_some("session")
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

impl From<Token> for Credentials {
    /// The token alone, with no narrower one.
    fn from(token: Token) -> Self {
        Self {
            token,
            read_token: None,
            consumer_token: None,
        }
    }
}

impl Credentials {
    /// The credential the request presents for the operation at `method` and `path`, if its
    /// role reaches that operation.
    #[must_use]
    pub fn admit(
        &self,
        request: &Request,
        method: &Method,
        path: &str,
    ) -> Option<CredentialIdentity> {
        [
            (Role::Admin, Some(&self.token)),
            (Role::Read, self.read_token.as_ref()),
            (Role::Consumer, self.consumer_token.as_ref()),
        ]
        .into_iter()
        .filter(|(role, _)| role.reaches(method, path))
        .find_map(|(role, token)| {
            Some(CredentialIdentity {
                credential_id: role.credential_id(),
                transport: token?.presented(request)?,
            })
        })
    }
}

impl Role {
    /// The narrower tokens reach no `POST` but the consumer's [`CONSUMER_ACTIONS`], so a
    /// dashboard or a service that holds one cannot change prices, plans or credentials.
    fn reaches(self, method: &Method, path: &str) -> bool {
        match self {
            Self::Admin => true,
            Self::Read => method == Method::GET,
            Self::Consumer => {
                (method == Method::GET && CONSUMER_READS.contains(&path))
                    || (method == Method::POST && CONSUMER_ACTIONS.contains(&path))
            }
        }
    }

    const fn credential_id(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Read => "read",
            Self::Consumer => "consumer",
        }
    }
}

/// Answers 401 to a request the [`Credentials`] do not admit.
pub struct RequireCredential {
    pub credentials: Credentials,
}

impl<E: Endpoint> Middleware<E> for RequireCredential {
    type Output = RequireCredentialEndpoint<E>;

    fn transform(&self, inner: E) -> Self::Output {
        RequireCredentialEndpoint {
            inner,
            credentials: self.credentials.clone(),
        }
    }
}

pub struct RequireCredentialEndpoint<E> {
    inner: E,
    credentials: Credentials,
}

impl<E: Endpoint> Endpoint for RequireCredentialEndpoint<E> {
    type Output = Response;

    async fn call(&self, mut request: Request) -> poem::Result<Self::Output> {
        if is_open(&request) {
            return Ok(self.inner.call(request).await?.into_response());
        }
        let Some(identity) =
            self.credentials
                .admit(&request, request.method(), request.uri().path())
        else {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .content_type("application/json")
                .body(serde_json::json!({ "message": "a credential is required" }).to_string()));
        };
        request.extensions_mut().insert(identity);

        Ok(self.inner.call(request).await?.into_response())
    }
}

/// The middleware is applied inside the nest at [`crate::http::MOUNT`], which has already taken the
/// prefix off the path it sees.
fn is_open(request: &Request) -> bool {
    let path = request.uri().path();

    OPEN.iter()
        .any(|(method, open)| request.method() == method && path == *open)
}
