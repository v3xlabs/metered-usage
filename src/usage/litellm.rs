//! One row of LiteLLM's `/spend/logs/v2`, and what it means here.
//!
//! LiteLLM keeps a row per request in its own database and serves it for as long as it
//! keeps it, so a row can be read many times; the record hash makes the second read a
//! duplicate. Cache token counts have no column of their own and live in `metadata`.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::Value;

use crate::account::{AuthKind, NewAccount, mask};
use crate::usage::NewUsageEvent;
use crate::usage::ingest::Incoming;

/// LiteLLM hashes the virtual key before it stores it, and a hash is still the key's
/// identity, so neither form is kept in a dead letter.
const KEY_FIELDS: [&str; 1] = ["api_key"];
const METADATA_KEY_FIELDS: [&str; 1] = ["user_api_key"];
const FALLBACK_STATUS: i64 = 500;

#[derive(Debug, Deserialize)]
pub struct LiteLlmRow {
    pub request_id: Option<String>,
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub spend: Option<f64>,
    #[serde(default)]
    pub total_tokens: Option<i64>,
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,
    #[serde(rename = "completionStartTime", default)]
    pub completion_start_time: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub request_duration_ms: Option<i64>,
    /// An object, or the same object as JSON text, depending on how the proxy serialized it.
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LiteLlmMetadata {
    #[serde(default)]
    pub user_api_key_alias: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub additional_usage_values: Option<LiteLlmAdditionalUsage>,
    #[serde(default)]
    pub error_information: Option<LiteLlmErrorInformation>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LiteLlmAdditionalUsage {
    #[serde(default)]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LiteLlmErrorInformation {
    /// A number or the number as text.
    #[serde(default)]
    pub error_code: Option<Value>,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LiteLlmError {
    #[error("the row is not a spend log: {0}")]
    Shape(#[from] serde_json::Error),
    #[error("the row has no request_id")]
    MissingRequestId,
    #[error("the row has no startTime")]
    MissingStartTime,
    #[error("{field} {value:?} is not an instant")]
    Timestamp { field: &'static str, value: String },
}

impl TryFrom<LiteLlmRow> for NewUsageEvent {
    type Error = LiteLlmError;

    fn try_from(row: LiteLlmRow) -> Result<Self, Self::Error> {
        let upstream_id = present(row.request_id).ok_or(LiteLlmError::MissingRequestId)?;
        let start_time = present(row.start_time).ok_or(LiteLlmError::MissingStartTime)?;
        let occurred_at = instant("startTime", &start_time)?;
        let ttft_ms = present(row.completion_start_time)
            .map(|text| instant("completionStartTime", &text))
            .transpose()?
            .map(|first_token| first_token.duration_since(occurred_at))
            .filter(|elapsed| !elapsed.is_negative())
            .map(|elapsed| elapsed.as_millis())
            .and_then(|milliseconds| i64::try_from(milliseconds).ok());
        let metadata = match row.metadata {
            Some(Value::String(text)) if !text.is_empty() => serde_json::from_str(&text)?,
            Some(Value::Null | Value::String(_)) | None => LiteLlmMetadata::default(),
            Some(value) => LiteLlmMetadata::deserialize(value)?,
        };

        let provider = present(row.custom_llm_provider).unwrap_or_else(|| "unknown".to_owned());
        let model = present(row.model).unwrap_or_default();
        let model_group = present(row.model_group);
        let upstream_key = present(row.model_id)
            .or_else(|| present(row.api_base))
            .unwrap_or_else(|| format!("unattributed/{provider}"));
        let account = NewAccount {
            upstream_key,
            auth_kind: AuthKind::ApiKey,
            label: model_group
                .clone()
                .or_else(|| Some(model.clone()).filter(|model| !model.is_empty())),
        };
        let caller =
            present(metadata.user_api_key_alias).or_else(|| row.api_key.as_deref().and_then(mask));
        let usage = metadata.additional_usage_values.unwrap_or_default();
        let failed = row
            .status
            .as_deref()
            .is_some_and(|status| status != "success");
        let error = metadata.error_information.unwrap_or_default();
        let reported_status = error.error_code.as_ref().and_then(|code| match code {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        });
        let error_message = present(error.error_message);
        let (status_code, error_message) = match (failed, reported_status) {
            (false, _) => (200, None),
            (true, Some(code)) => (code, error_message),
            (true, None) => {
                let note = format!("LiteLLM recorded no status code, stored as {FALLBACK_STATUS}");
                (
                    FALLBACK_STATUS,
                    Some(
                        error_message.map_or(note.clone(), |message| format!("{message} ({note})")),
                    ),
                )
            }
        };
        let prompt_tokens = row.prompt_tokens.unwrap_or_default();
        let completion_tokens = row.completion_tokens.unwrap_or_default();

        Ok(Self {
            upstream_id,
            occurred_at,
            provider,
            account,
            model_alias: model_group.filter(|group| *group != model),
            model,
            endpoint: present(row.call_type).unwrap_or_default(),
            caller,
            harness: None,
            user_agent: present(metadata.user_agent),
            session_id: present(row.session_id),
            input_tokens: prompt_tokens,
            output_tokens: completion_tokens,
            reasoning_tokens: 0,
            cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
            cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
            unclassified_tokens: 0,
            total_tokens: row
                .total_tokens
                .unwrap_or(prompt_tokens + completion_tokens),
            token_quality: None,
            latency_ms: row.request_duration_ms,
            ttft_ms,
            streamed: false,
            status_code,
            failed,
            error_message,
            service_tier: None,
            reasoning_effort: None,
            billed_cost_usd: row.spend,
        })
    }
}

/// A row this service cannot store is dead-lettered without the key hash it carries.
pub fn parse(row: Value) -> Incoming {
    let event = LiteLlmRow::deserialize(&row)
        .map_err(LiteLlmError::from)
        .and_then(NewUsageEvent::try_from);
    match event {
        Ok(event) => Incoming::Record(Box::new(event)),
        Err(error) => Incoming::Rejected {
            error: error.to_string(),
            payload: stripped(row),
        },
    }
}

fn stripped(mut row: Value) -> Value {
    if let Value::Object(fields) = &mut row {
        for field in KEY_FIELDS {
            fields.remove(field);
        }
        if let Some(metadata) = fields.get_mut("metadata") {
            if let Value::String(text) = metadata {
                *metadata = serde_json::from_str(text).unwrap_or(Value::Null);
            }
            if let Value::Object(metadata) = metadata {
                for field in METADATA_KEY_FIELDS {
                    metadata.remove(field);
                }
            }
        }
    }
    row
}

/// LiteLLM stores `startTime` as UTC and may print it without an offset.
fn instant(field: &'static str, text: &str) -> Result<Timestamp, LiteLlmError> {
    text.parse::<Timestamp>()
        .or_else(|_| {
            text.parse::<DateTime>()
                .and_then(|civil| civil.to_zoned(TimeZone::UTC))
                .map(|zoned| zoned.timestamp())
        })
        .map_err(|_| LiteLlmError::Timestamp {
            field,
            value: text.to_owned(),
        })
}

fn present(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}
