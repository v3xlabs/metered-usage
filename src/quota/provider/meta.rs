//! Meta: `muse-code/key`, authorized with the credential file's `dca_token` rather than
//! CLIProxy's token substitution. The file is downloaded for that one call; neither it nor
//! the token outlives the call, and no message quotes either or the answer, which echoes
//! the API key.

use serde_json::Value;

use crate::quota::management::{ApiCall, AuthFile, Management};
use crate::quota::provider::{
    Observation, RefreshError, answer, instant, integer, number, text, used_from_percent,
};
use crate::quota::window::NewWindow;

const KEY_URL: &str = "https://api.meta.ai/muse-code/key";
const WEEK: i64 = 7 * 24 * 60 * 60;
const DCA_PREFIX: &str = "dca:";

/// A runtime-only credential has no file to download.
#[must_use]
pub fn has_file(file: &AuthFile) -> bool {
    !file.entry.runtime_only
        && file
            .entry
            .name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
}

pub async fn fetch(
    management: &Management<'_>,
    file: &AuthFile,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    let name = file
        .entry
        .name
        .as_deref()
        .filter(|_| has_file(file))
        .ok_or(RefreshError::MissingFile)?;
    let token = dca_token(&management.download(name).await?)?;
    let header = [
        ("Accept", "application/json".to_owned()),
        ("Content-Type", "application/json".to_owned()),
        ("Authorization", format!("Bearer {token}")),
        ("x-api-version", "1.0.0".to_owned()),
    ];
    let key = answer(
        management
            .call(&ApiCall {
                auth_index,
                method: "POST",
                url: KEY_URL,
                header: &header,
                data: Some("{}"),
            })
            .await?,
    )?;
    if !key.is_object() {
        return Err(RefreshError::Unreadable);
    }

    Ok(read(&key))
}

/// Only the persisted DCA field, never the LLM key beside it.
fn dca_token(credential: &str) -> Result<String, RefreshError> {
    let credential = serde_json::from_str::<Value>(credential)
        .ok()
        .filter(Value::is_object)
        .ok_or(RefreshError::InvalidCredentialFile)?;
    credential
        .get("dca_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| {
            token
                .strip_prefix(DCA_PREFIX)
                .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
        })
        .map(str::to_owned)
        .ok_or(RefreshError::MissingDcaToken)
}

/// Reads the quota fields by name and nothing else of an answer that also carries the key.
fn read(key: &Value) -> Observation {
    let usage = key.get("subs_usage").unwrap_or(&Value::Null);
    let window = |name: &str| usage.get(name).filter(|window| window.is_object());
    let mut windows = Vec::new();
    if let Some(rolling) = window("window") {
        push(
            &mut windows,
            rolling,
            "window",
            "Rolling window",
            rolling
                .get("window_duration_mins")
                .and_then(integer)
                .filter(|minutes| *minutes > 0)
                .and_then(|minutes| minutes.checked_mul(60)),
        );
    }
    if let Some(weekly) = window("weekly") {
        push(&mut windows, weekly, "weekly", "Weekly", Some(WEEK));
    }
    let plan = key
        .get("subs_tier_name")
        .and_then(text)
        .or_else(|| usage.get("tier").and_then(text));

    Observation { windows, plan }
}

fn push(
    windows: &mut Vec<NewWindow>,
    window: &Value,
    key: &str,
    label: &str,
    window_seconds: Option<i64>,
) {
    let used = window
        .get("used_percent")
        .and_then(number)
        .map(used_from_percent);
    let resets_at = window
        .get("resets_at")
        .filter(|reset| number(reset).is_some_and(|reset| reset > 0.0))
        .and_then(instant);
    if used.is_some() || resets_at.is_some() {
        windows.push(NewWindow {
            key: key.to_owned(),
            label: label.to_owned(),
            used_fraction: used,
            window_seconds,
            resets_at,
            ..NewWindow::default()
        });
    }
}
