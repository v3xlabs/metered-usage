//! One adapter per provider the official CLIProxy management UI
//! (`router-for-me/Cli-Proxy-API-Management-Center`) can read quota for. Each sends the
//! UI's own `api-call` requests, byte for byte, and normalizes the answer.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod devin;
pub mod kimi;
pub mod meta;
pub mod xai;

use std::str::FromStr;

use jiff::{SignedDuration, Timestamp};
use serde_json::Value;

use crate::quota::management::{ApiResponse, AuthFile, Management, ManagementError};
use crate::quota::window::NewWindow;

const PERCENT: f64 = 100.0;
/// Below this a Unix instant is in seconds, above it in milliseconds; the two ranges are
/// centuries apart.
const UNIX_MILLISECONDS_FROM: f64 = 1e11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
    Kimi,
    Devin,
    Xai,
    Meta,
}

impl Provider {
    /// Whether the entry carries what the adapter needs. The UI never refreshes a disabled
    /// credential either.
    #[must_use]
    pub fn can_refresh(self, file: &AuthFile) -> bool {
        let entry = &file.entry;
        if entry.disabled || entry.auth_index.as_deref().is_none_or(str::is_empty) {
            return false;
        }
        match self {
            Self::Claude | Self::Codex | Self::Kimi | Self::Devin | Self::Xai => true,
            Self::Antigravity => antigravity::project_id(file).is_some(),
            Self::Meta => meta::has_file(file),
        }
    }

    pub async fn fetch(
        self,
        management: &Management<'_>,
        file: &AuthFile,
        auth_index: &str,
    ) -> Result<Observation, RefreshError> {
        match self {
            Self::Claude => claude::fetch(management, auth_index).await,
            Self::Codex => codex::fetch(management, file, auth_index).await,
            Self::Antigravity => antigravity::fetch(management, file, auth_index).await,
            Self::Kimi => kimi::fetch(management, auth_index).await,
            Self::Devin => devin::fetch(management, auth_index).await,
            Self::Xai => xai::fetch(management, file, auth_index).await,
            Self::Meta => meta::fetch(management, file, auth_index).await,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("no quota adapter for provider {0:?}")]
pub struct UnsupportedProvider(pub String);

impl FromStr for Provider {
    type Err = UnsupportedProvider;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "antigravity" => Ok(Self::Antigravity),
            "kimi" => Ok(Self::Kimi),
            "devin" => Ok(Self::Devin),
            "xai" => Ok(Self::Xai),
            "meta" => Ok(Self::Meta),
            other => Err(UnsupportedProvider(other.to_owned())),
        }
    }
}

/// What one hard refresh learned about a credential.
#[derive(Debug, Default)]
pub struct Observation {
    pub windows: Vec<NewWindow>,
    pub plan: Option<String>,
}

/// Every message is safe to store and to answer to a client: none quotes a response body.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error(transparent)]
    Management(#[from] ManagementError),
    #[error("the provider answered {0}")]
    Upstream(u16),
    #[error("the provider's answer was not the expected JSON")]
    Unreadable,
    #[error("the provider reported no quota")]
    Empty,
    #[error("the credential entry has no project_id")]
    MissingProjectId,
    #[error("the credential has no file to read")]
    MissingFile,
    #[error("the credential file is not a JSON object")]
    InvalidCredentialFile,
    #[error("the credential file has no dca_token")]
    MissingDcaToken,
}

/// The provider's answer as JSON, or why it is not one.
fn answer(response: ApiResponse) -> Result<Value, RefreshError> {
    let ApiResponse { status_code, body } = response;
    if !(200..300).contains(&status_code) {
        return Err(RefreshError::Upstream(status_code));
    }
    serde_json::from_str::<Value>(body.trim()).map_err(|_| RefreshError::Unreadable)
}

/// The first of `keys` present and not null, for payloads that spell a field two ways.
fn field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(key))
        .find(|found| !found.is_null())
}

/// A finite number, or a string holding one.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|number| number.is_finite())
}

/// A non-blank string, or a number written as one.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim())
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// An RFC 3339 instant, or Unix time in seconds or milliseconds.
fn instant(value: &Value) -> Option<Timestamp> {
    if let Some(parsed) = value
        .as_str()
        .and_then(|text| text.trim().parse::<Timestamp>().ok())
    {
        return Some(parsed);
    }
    let unix = number(value).filter(|unix| *unix > 0.0)?;
    let seconds = if unix < UNIX_MILLISECONDS_FROM {
        unix
    } else {
        unix / 1000.0
    };
    offset(Timestamp::UNIX_EPOCH, seconds)
}

/// An instant given as seconds from `now`.
fn after_seconds(value: &Value, now: Timestamp) -> Option<Timestamp> {
    offset(now, number(value).filter(|seconds| *seconds > 0.0)?)
}

fn offset(from: Timestamp, seconds: f64) -> Option<Timestamp> {
    from.checked_add(SignedDuration::try_from_secs_f64(seconds).ok()?)
        .ok()
}

/// A whole number, or a string holding one.
fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn used_from_percent(percent: f64) -> f64 {
    (percent / PERCENT).clamp(0.0, 1.0)
}

fn truthy(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => number.as_f64().map(|number| number != 0.0),
        Value::String(text) => match text.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Some(true),
            "false" | "0" | "no" | "n" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A label reduced to lowercase words joined by dashes, as the UI derives window ids.
fn slug(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The credential entry and the objects the UI also looks in for the same field.
fn entry_records(file: &AuthFile) -> [Option<&Value>; 3] {
    [
        Some(&file.raw),
        file.raw.get("metadata").filter(|value| value.is_object()),
        file.raw.get("attributes").filter(|value| value.is_object()),
    ]
}
