//! Antigravity: `retrieveUserQuotaSummary` tried on three hosts in order until one answers,
//! and `loadCodeAssist` for the plan. Buckets report the fraction remaining.

use serde_json::Value;

use crate::quota::management::{ApiCall, AuthFile, Management};
use crate::quota::provider::{
    Observation, RefreshError, answer, entry_records, field, instant, number, slug, text,
};
use crate::quota::window::NewWindow;

const QUOTA_URLS: [&str; 3] = [
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
];
const CODE_ASSIST_URL: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const CODE_ASSIST_BODY: &str = r#"{"metadata":{"ideType":"ANTIGRAVITY"}}"#;
const USER_AGENT: &str = "antigravity/cli/1.0.13 (aidev_client; os_type=darwin; arch=arm64)";
const FIVE_HOURS: i64 = 5 * 60 * 60;
const WEEK: i64 = 7 * 24 * 60 * 60;

/// The Google Cloud project the quota is read for. The UI would download the credential
/// file when the entry does not name one; this service reads a credential file for Meta
/// alone, so such a credential is not hard refreshable.
#[must_use]
pub fn project_id(file: &AuthFile) -> Option<String> {
    if let Some(project) = file
        .entry
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|project| !project.is_empty())
    {
        return Some(project.to_owned());
    }
    let [_, metadata, attributes] = entry_records(file);
    metadata
        .and_then(|metadata| field(metadata, &["project_id", "projectId"]))
        .or_else(|| {
            attributes.and_then(|attributes| {
                field(
                    attributes,
                    &["project_id", "projectId", "gemini_virtual_project"],
                )
            })
        })
        .and_then(text)
}

pub async fn fetch(
    management: &Management<'_>,
    file: &AuthFile,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    let project = project_id(file).ok_or(RefreshError::MissingProjectId)?;
    let body = serde_json::json!({ "project": project }).to_string();
    let header = [
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("Content-Type", "application/json".to_owned()),
        ("User-Agent", USER_AGENT.to_owned()),
    ];
    let subscription = async {
        let response = management
            .call(&ApiCall {
                auth_index,
                method: "POST",
                url: CODE_ASSIST_URL,
                header: &header,
                data: Some(CODE_ASSIST_BODY),
            })
            .await
            .ok()?;
        plan(&unwrap_body(answer(response).ok()?))
    };
    let quota = async {
        let mut failure = None;
        let mut answered = false;
        for url in QUOTA_URLS {
            let outcome = management
                .call(&ApiCall {
                    auth_index,
                    method: "POST",
                    url,
                    header: &header,
                    data: Some(&body),
                })
                .await
                .map_err(RefreshError::from)
                .and_then(answer);
            match outcome {
                Ok(payload) => {
                    answered = true;
                    let windows = windows(&unwrap_body(payload));
                    if !windows.is_empty() {
                        return Ok(windows);
                    }
                }
                Err(error) => {
                    // 403 and 404 say most about the credential, so they outrank a later
                    // host's failure.
                    let decisive = matches!(failure, Some(RefreshError::Upstream(403 | 404)));
                    if !decisive {
                        failure = Some(error);
                    }
                }
            }
        }
        if answered {
            Ok(Vec::new())
        } else {
            Err(failure.unwrap_or(RefreshError::Empty))
        }
    };
    let (plan, windows) = tokio::join!(subscription, quota);

    Ok(Observation {
        windows: windows?,
        plan,
    })
}

/// Some answers wrap the payload in a `body` object.
fn unwrap_body(payload: Value) -> Value {
    if payload.get("models").is_some() {
        return payload;
    }
    match payload.get("body") {
        Some(Value::Object(body)) => Value::Object(body.clone()),
        Some(Value::String(body)) => serde_json::from_str::<Value>(body)
            .ok()
            .filter(Value::is_object)
            .unwrap_or(payload),
        _ => payload,
    }
}

fn windows(payload: &Value) -> Vec<NewWindow> {
    let groups = payload
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut windows = Vec::new();
    for (group_index, group) in groups.enumerate() {
        let group_label = field(group, &["displayName", "display_name"])
            .and_then(text)
            .unwrap_or_else(|| format!("Quota Group {}", group_index + 1));
        let group_id = Some(slug(&group_label))
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| format!("quota-group-{}", group_index + 1));
        let buckets = group
            .get("buckets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for (bucket_index, bucket) in buckets.enumerate() {
            let Some(remaining) =
                field(bucket, &["remainingFraction", "remaining_fraction"]).and_then(fraction)
            else {
                continue;
            };
            let window = bucket.get("window").and_then(text);
            let id = field(bucket, &["bucketId", "bucket_id"])
                .and_then(text)
                .unwrap_or_else(|| match &window {
                    Some(window) => format!("{group_id}-{window}"),
                    None => format!("{group_id}-bucket-{}", bucket_index + 1),
                });
            let label = field(bucket, &["displayName", "display_name"])
                .and_then(text)
                .unwrap_or_else(|| id.clone());
            windows.push(NewWindow {
                key: id,
                label: format!("{group_label}: {label}"),
                used_fraction: Some(1.0 - remaining.clamp(0.0, 1.0)),
                window_seconds: window.as_deref().and_then(window_seconds),
                resets_at: field(bucket, &["resetTime", "reset_time"]).and_then(instant),
                ..NewWindow::default()
            });
        }
    }

    windows
}

/// A fraction as a number, a numeric string, or a percentage string such as `"40%"`.
fn fraction(value: &Value) -> Option<f64> {
    number(value).or_else(|| {
        value
            .as_str()?
            .trim()
            .strip_suffix('%')?
            .parse::<f64>()
            .ok()
            .filter(|percent| percent.is_finite())
            .map(|percent| percent / 100.0)
    })
}

fn window_seconds(window: &str) -> Option<i64> {
    match window.trim().to_lowercase().as_str() {
        "5h" | "five-hour" | "five_hour" => Some(FIVE_HOURS),
        "weekly" | "week" => Some(WEEK),
        _ => None,
    }
}

/// A paid tier outranks the current one.
fn plan(payload: &Value) -> Option<String> {
    let tier = |keys: &[&str]| {
        let tier = field(payload, keys)?;
        Some((
            tier.get("id").and_then(text),
            tier.get("name").and_then(text),
        ))
    };
    let paid = tier(&["paidTier", "paid_tier"]).filter(|(id, _)| id.is_some());
    let (id, name) = paid.or_else(|| tier(&["currentTier", "current_tier"]))?;
    match id.as_deref() {
        Some("free-tier") => Some("free".to_owned()),
        Some("g1-pro-tier") => Some("pro".to_owned()),
        Some("g1-ultra-tier") => Some("ultra".to_owned()),
        Some("g1-ultra-lite-tier") => Some("ultra-lite".to_owned()),
        _ => id.or(name),
    }
}
