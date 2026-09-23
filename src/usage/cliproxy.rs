//! The accounting record `CLIProxyAPI` writes, and what it means here.
//!
//! Upstream keeps these records in a ring buffer that a read destructively pops, so a
//! record reaches this service once. Fields it does not know are absent, fields this
//! service does not use are ignored.

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::Value;

use crate::account::{self, AuthKind, NewAccount};
use crate::usage::ingest::Incoming;
use crate::usage::{NewUsageEvent, TokenQuality};

/// Providers whose legacy input holds its cache and whose reasoning is apart from output.
const SEPARATE_REASONING_MARKERS: [&str; 5] =
    ["gemini", "aistudio", "antigravity", "vertex", "interaction"];
/// Providers whose legacy input holds its cache and whose output holds its reasoning.
const SUBSET_MARKERS: [&str; 8] = [
    "openai",
    "codex",
    "xai",
    "grok",
    "kimi",
    "qwen",
    "deepseek",
    "openrouter",
];

#[derive(Debug, Deserialize)]
pub struct CliProxyRecord {
    pub timestamp: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub executor_type: Option<String>,
    pub endpoint: String,
    pub tokens: CliProxyTokens,
    pub fail: CliProxyFail,
    #[serde(default)]
    pub accounting_version: i64,
    #[serde(default)]
    pub token_breakdown: Option<CliProxyBreakdown>,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub latency_ms: Option<i64>,
    #[serde(default)]
    pub ttft_ms: Option<i64>,
    /// The login email for OAuth, and the raw API key for an API-key credential, so it is
    /// read only to label the account and never kept as it arrived.
    #[serde(default)]
    pub source: Option<String>,
    /// The gateway's name for the credential file, stable across its restarts.
    #[serde(default)]
    pub auth_index: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    /// The raw key the client presented to the gateway.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub auth_type: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// What the client asked for, such as `auto`.
    #[serde(default)]
    pub service_tier: Option<String>,
    /// What the upstream says it served, which is the tier it bills.
    #[serde(default)]
    pub response_service_tier: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CliProxyTokens {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub cached_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub cache_creation_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

#[derive(Debug, Deserialize)]
pub struct CliProxyFail {
    #[serde(default)]
    pub status_code: i64,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct CliProxyBreakdown {
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub unclassified_tokens: i64,
    pub input: CliProxyInputBreakdown,
    pub output: CliProxyOutputBreakdown,
}

#[derive(Debug, Deserialize)]
pub struct CliProxyInputBreakdown {
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
}

#[derive(Debug, Deserialize)]
pub struct CliProxyOutputBreakdown {
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CliProxyError {
    #[error("timestamp {0:?} is not an instant")]
    Timestamp(String),
    #[error("accounting version {version} promises a token breakdown and the record has none")]
    MissingBreakdown { version: i64 },
}

impl TryFrom<CliProxyRecord> for NewUsageEvent {
    type Error = CliProxyError;

    fn try_from(record: CliProxyRecord) -> Result<Self, Self::Error> {
        let occurred_at = record
            .timestamp
            .parse::<Timestamp>()
            .map_err(|_| CliProxyError::Timestamp(record.timestamp.clone()))?;
        // Version 2 splits every bucket apart the same way for every provider. The flat
        // block below it is each provider's own count.
        let tokens = if record.accounting_version >= 2 {
            let breakdown = record
                .token_breakdown
                .ok_or(CliProxyError::MissingBreakdown {
                    version: record.accounting_version,
                })?;
            Tokens {
                input: breakdown.input.total_tokens,
                output: breakdown.output.total_tokens,
                reasoning: breakdown.output.reasoning_tokens,
                cache_read: breakdown.input.cache_read_tokens,
                cache_write: breakdown.input.cache_write_tokens,
                unclassified: breakdown.unclassified_tokens,
                total: breakdown.total_tokens,
                // A verdict this service does not know says nothing it can store.
                quality: breakdown
                    .quality
                    .as_deref()
                    .and_then(|quality| quality.parse().ok()),
            }
        } else {
            Tokens::flat(
                &record.tokens,
                Semantics::of(
                    &record.provider,
                    record.executor_type.as_deref().unwrap_or_default(),
                ),
            )
        };
        let auth_kind = auth_kind(record.auth_type.as_deref());
        let harness = record
            .user_agent
            .as_deref()
            .and_then(|agent| agent.split_whitespace().next())
            .and_then(|product| product.split('/').next())
            .filter(|product| !product.is_empty())
            .map(str::to_owned);

        let account = NewAccount::new(
            record.auth_index.unwrap_or_default(),
            auth_kind,
            record.source.as_deref(),
        );

        Ok(Self {
            upstream_id: record.request_id.unwrap_or_default(),
            occurred_at,
            provider: record.provider,
            model: record.model,
            model_alias: present(record.alias),
            endpoint: record.endpoint,
            account,
            caller: record.api_key.as_deref().and_then(account::mask),
            harness,
            user_agent: present(record.user_agent),
            session_id: present(record.session_id),
            input_tokens: tokens.input,
            output_tokens: tokens.output,
            reasoning_tokens: tokens.reasoning,
            cache_read_tokens: tokens.cache_read,
            cache_write_tokens: tokens.cache_write,
            unclassified_tokens: tokens.unclassified,
            total_tokens: tokens.total,
            token_quality: tokens.quality,
            latency_ms: record.latency_ms,
            ttft_ms: record.ttft_ms,
            streamed: record.stream,
            status_code: record.fail.status_code,
            // The body of a request that did not fail is the upstream's idea of nothing.
            failed: record.failed,
            error_message: record
                .failed
                .then_some(record.fail.body)
                .filter(|body| !body.is_empty()),
            service_tier: present(record.response_service_tier)
                .or_else(|| present(record.service_tier)),
            reasoning_effort: present(record.reasoning_effort),
            billed_cost_usd: None,
        })
    }
}

/// Which of the two token blocks was read.
struct Tokens {
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    unclassified: i64,
    total: i64,
    quality: Option<TokenQuality>,
}

impl Tokens {
    /// Reads a record from a gateway older than the breakdown the way the gateway itself
    /// now reads such a record, `EnsureTokenBreakdownForProvider` in CLIProxyAPI's
    /// `sdk/cliproxy/usage/accounting.go`. Such a gateway sent Anthropic's input without its
    /// cache, and the cache reads of Codex and Gemini only as `cached_tokens`.
    fn flat(tokens: &CliProxyTokens, semantics: Semantics) -> Self {
        let cache_read = if tokens.cache_read_tokens > 0 {
            tokens.cache_read_tokens
        } else {
            tokens.cached_tokens
        };
        let (input, cache_read, output) = match semantics {
            // Anthropic's output already holds its thinking.
            Semantics::Independent => (
                tokens.input_tokens + tokens.cache_read_tokens + tokens.cache_creation_tokens,
                tokens.cache_read_tokens,
                tokens.output_tokens,
            ),
            Semantics::SeparateReasoning => (
                tokens.input_tokens,
                cache_read,
                tokens.output_tokens + tokens.reasoning_tokens,
            ),
            Semantics::Subset => (tokens.input_tokens, cache_read, tokens.output_tokens),
            Semantics::Unknown => {
                return Self {
                    input: 0,
                    output: 0,
                    reasoning: 0,
                    cache_read: 0,
                    cache_write: 0,
                    unclassified: tokens.total_tokens,
                    total: tokens.total_tokens,
                    quality: Some(TokenQuality::Unclassified),
                };
            }
        };

        Self {
            input,
            output,
            reasoning: tokens.reasoning_tokens,
            cache_read,
            cache_write: tokens.cache_creation_tokens,
            unclassified: 0,
            total: tokens.total_tokens,
            quality: None,
        }
    }
}

/// How one provider's flat token counts overlap, as `tokenAccountingSemanticsFor` in
/// CLIProxyAPI's `sdk/cliproxy/usage/accounting.go` names it.
#[derive(Debug, Clone, Copy)]
enum Semantics {
    /// Input holds its cache reads and writes; output holds its reasoning.
    Subset,
    /// Input, cache reads and cache writes are apart.
    Independent,
    /// Input holds its cache reads; reasoning is apart from output.
    SeparateReasoning,
    /// The gateway does not guess, so neither does this service.
    Unknown,
}

impl Semantics {
    fn of(provider: &str, executor: &str) -> Self {
        let provider = provider.trim().to_lowercase();
        let executor = executor.trim().to_lowercase();
        let named = format!("{provider} {executor}");
        if executor == "openaicompatexecutor"
            || provider == "openai-compatibility"
            || provider.starts_with("openai-compatible-")
        {
            Self::Subset
        } else if named.contains("claude") || named.contains("anthropic") {
            Self::Independent
        } else if SEPARATE_REASONING_MARKERS
            .iter()
            .any(|marker| named.contains(marker))
        {
            Self::SeparateReasoning
        } else if SUBSET_MARKERS.iter().any(|marker| named.contains(marker)) {
            Self::Subset
        } else {
            Self::Unknown
        }
    }
}

/// Reads one record as the gateway sent it. A record that cannot be read is rejected with
/// the fields that can carry a credential removed: the client key, the response headers,
/// and the credential source unless it is an OAuth login.
#[must_use]
pub fn parse(value: Value) -> Incoming {
    let read = CliProxyRecord::deserialize(&value)
        .map_err(|error| error.to_string())
        .and_then(|record| NewUsageEvent::try_from(record).map_err(|error| error.to_string()));
    match read {
        Ok(event) => Incoming::Record(Box::new(event)),
        Err(error) => Incoming::Rejected {
            error,
            payload: stripped(value),
        },
    }
}

fn stripped(mut value: Value) -> Value {
    if let Value::Object(fields) = &mut value {
        fields.remove("api_key");
        fields.remove("response_headers");
        let oauth = fields
            .get("auth_type")
            .and_then(Value::as_str)
            .is_some_and(|auth_type| auth_kind(Some(auth_type)) == AuthKind::OAuth);
        if !oauth {
            fields.remove("source");
        }
    }

    value
}

fn auth_kind(auth_type: Option<&str>) -> AuthKind {
    match auth_type.unwrap_or_default() {
        "oauth" | "oauth2" => AuthKind::OAuth,
        "apikey" | "api_key" | "api-key" => AuthKind::ApiKey,
        _ => AuthKind::Unknown,
    }
}

/// A field the gateway sent empty said nothing, and is stored as nothing.
fn present(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}
