//! CLIProxy's management API: the credential list it keeps in memory, and the proxy through
//! which a provider is called with one credential's token.
//!
//! Reading the list never reaches a provider. Every `api-call` does, and CLIProxy keeps
//! nothing of its answer.

use std::time::Duration;

use jiff::Timestamp;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::config::{Secret, SourceConfig};
use crate::quota::credential::Cooldown;

/// Bounds every request to CLIProxy, so a provider that never answers fails its credential
/// instead of the whole refresh.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const PREFIX: &str = "/v0/management";

pub struct Management<'a> {
    client: &'a Client,
    base_url: &'a str,
    secret: &'a Secret,
}

impl<'a> Management<'a> {
    #[must_use]
    pub fn new(client: &'a Client, source: &'a SourceConfig) -> Self {
        Self {
            client,
            base_url: &source.base_url,
            secret: &source.secret,
        }
    }

    /// `GET /v0/management/auth-files`, which CLIProxy answers from memory.
    pub async fn auth_files(&self) -> Result<AuthFiles, ManagementError> {
        let response = self
            .client
            .get(format!("{}{PREFIX}/auth-files", self.base_url))
            .bearer_auth(self.secret.expose())
            .timeout(CALL_TIMEOUT)
            .send()
            .await
            .map_err(ManagementError::from)?;
        let listing: Listing = successful(response)?
            .json()
            .await
            .map_err(ManagementError::from)?;

        let raw_files = listing.files;
        let mut is_complete = raw_files.is_some();
        let files = raw_files
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .filter_map(|(position, raw)| {
                // The serde message can quote the offending value, which may be a key.
                if let Ok(entry) = Entry::deserialize(&raw) {
                    Some(AuthFile { entry, raw })
                } else {
                    tracing::warn!(position, "skipped an unreadable auth-files entry");
                    is_complete = false;
                    None
                }
            })
            .collect();

        Ok(AuthFiles {
            observed_at: listing.observed_at.unwrap_or_else(Timestamp::now),
            files,
            is_complete,
        })
    }

    /// `POST /v0/management/api-call`: CLIProxy puts the credential's token where the call
    /// says `$TOKEN$`, sends it to the provider and answers what the provider said.
    pub async fn call(&self, call: &ApiCall<'_>) -> Result<ApiResponse, ManagementError> {
        let response = self
            .client
            .post(format!("{}{PREFIX}/api-call", self.base_url))
            .bearer_auth(self.secret.expose())
            .json(call)
            .timeout(CALL_TIMEOUT)
            .send()
            .await
            .map_err(ManagementError::from)?;

        successful(response)?
            .json()
            .await
            .map_err(ManagementError::from)
    }

    /// `GET /v0/management/auth-files/download`: the whole credential file, secrets
    /// included. The caller keeps it only as long as the one request that needs it.
    pub async fn download(&self, name: &str) -> Result<String, ManagementError> {
        let mut url = Url::parse(&format!("{}{PREFIX}/auth-files/download", self.base_url))
            .map_err(|_| ManagementError::InvalidBaseUrl)?;
        url.query_pairs_mut().append_pair("name", name);
        let response = self
            .client
            .get(url)
            .bearer_auth(self.secret.expose())
            .timeout(CALL_TIMEOUT)
            .send()
            .await
            .map_err(ManagementError::from)?;

        successful(response)?
            .text()
            .await
            .map_err(ManagementError::from)
    }

    pub async fn reset_cooldown(&self, auth_index: &str) -> Result<(), ManagementError> {
        let response = self
            .client
            .post(format!("{}{PREFIX}/reset-quota", self.base_url))
            .bearer_auth(self.secret.expose())
            .json(&CooldownReset { auth_index })
            .timeout(CALL_TIMEOUT)
            .send()
            .await
            .map_err(ManagementError::from)?;
        let answer: CooldownResetAnswer = successful(response)?
            .json()
            .await
            .map_err(ManagementError::from)?;
        if answer.status != "ok" || answer.auth_index != auth_index {
            return Err(ManagementError::CooldownKept);
        }

        Ok(())
    }
}

pub struct AuthFiles {
    /// When CLIProxy read its own state, not when this service received it.
    pub observed_at: Timestamp,
    pub files: Vec<AuthFile>,
    /// Whether `files` holds every credential CLIProxy knows: the list was present and every
    /// entry could be read. Only a complete listing can tell that a credential is gone, so
    /// `store` also treats an entry without an `auth_index` as incomplete.
    pub is_complete: bool,
}

/// One credential as CLIProxy lists it. `raw` is the whole entry, kept in memory only,
/// for the few adapters that look past the typed fields.
pub struct AuthFile {
    pub entry: Entry,
    pub raw: Value,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub auth_index: Option<String>,
    pub name: Option<String>,
    pub provider: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub label: Option<String>,
    pub email: Option<String>,
    pub status: Option<String>,
    pub status_message: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub unavailable: bool,
    #[serde(default)]
    pub runtime_only: bool,
    pub next_retry_after: Option<Timestamp>,
    /// `null` when CLIProxy does not know its cooldowns, which is not the same as none.
    pub cooldowns: Option<Vec<Cooldown>>,
    pub account_type: Option<String>,
    pub project_id: Option<String>,
    /// Codex only: claims CLIProxy read from the credential's ID token.
    pub id_token: Option<Value>,
}

impl Entry {
    /// The provider name, lowercased, from `provider` or its duplicate `type`.
    #[must_use]
    pub fn provider(&self) -> Option<String> {
        [&self.provider, &self.kind]
            .into_iter()
            .flatten()
            .map(|name| name.trim())
            .find(|name| !name.is_empty())
            .map(str::to_lowercase)
    }
}

#[derive(Deserialize)]
struct Listing {
    observed_at: Option<Timestamp>,
    files: Option<Vec<Value>>,
}

#[derive(Serialize)]
struct CooldownReset<'a> {
    auth_index: &'a str,
}

#[derive(Deserialize)]
struct CooldownResetAnswer {
    #[serde(default)]
    status: String,
    #[serde(default)]
    auth_index: String,
}

/// The request body the official management UI sends to `api-call`.
#[derive(Debug, Serialize)]
pub struct ApiCall<'a> {
    #[serde(rename = "authIndex")]
    pub auth_index: &'a str,
    pub method: &'static str,
    pub url: &'a str,
    #[serde(serialize_with = "header_map")]
    pub header: &'a [(&'static str, String)],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<&'a str>,
}

/// What the provider answered. `body` is its raw text.
#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    pub status_code: u16,
    #[serde(default)]
    pub body: String,
}

/// Messages carry no body text and no URL: a provider may echo a key in an error body, and a
/// base URL may hold credentials.
#[derive(Debug, thiserror::Error)]
pub enum ManagementError {
    #[error("CLIProxy could not be reached: {0}")]
    Transport(reqwest::Error),
    #[error("CLIProxy answered {0}")]
    Status(u16),
    #[error("the source's base URL is not a URL")]
    InvalidBaseUrl,
    #[error("CLIProxy did not confirm that it cleared the credential's cooldowns")]
    CooldownKept,
}

impl From<reqwest::Error> for ManagementError {
    fn from(error: reqwest::Error) -> Self {
        Self::Transport(error.without_url())
    }
}

fn successful(response: reqwest::Response) -> Result<reqwest::Response, ManagementError> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else {
        Err(ManagementError::Status(status.as_u16()))
    }
}

fn header_map<S: Serializer>(
    header: &&[(&'static str, String)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(header.iter().map(|(name, value)| (name, value)))
}
