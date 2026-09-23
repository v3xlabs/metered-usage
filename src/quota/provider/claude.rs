//! Claude: `/api/oauth/usage` for the windows, `/api/oauth/profile` for the plan. The
//! profile is optional; the usage answer is not.

use serde_json::Value;

use crate::quota::management::{ApiCall, Management};
use crate::quota::provider::{
    Observation, RefreshError, answer, instant, number, text, truthy, used_from_percent,
};
use crate::quota::window::NewWindow;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const FIVE_HOURS: i64 = 5 * 60 * 60;
const WEEK: i64 = 7 * 24 * 60 * 60;
const FABLE_KEY: &str = "iguana_necktie";

/// Payload key, window key and label. Claude names its windows and never states their
/// length, so the length follows from the key: `five_hour` is five hours, every other one
/// a week.
const WINDOWS: [(&str, &str, &str); 7] = [
    ("five_hour", "five-hour", "5-hour"),
    ("seven_day", "seven-day", "7-day"),
    (
        "seven_day_oauth_apps",
        "seven-day-oauth-apps",
        "7-day OAuth apps",
    ),
    ("seven_day_opus", "seven-day-opus", "7-day Opus"),
    ("seven_day_sonnet", "seven-day-sonnet", "7-day Sonnet"),
    ("seven_day_cowork", "seven-day-cowork", "7-day Cowork"),
    (FABLE_KEY, "seven-day-fable", "7-day Fable"),
];

pub async fn fetch(
    management: &Management<'_>,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    let header = [
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("Content-Type", "application/json".to_owned()),
        ("anthropic-beta", "oauth-2025-04-20".to_owned()),
    ];
    let usage = ApiCall {
        auth_index,
        method: "GET",
        url: USAGE_URL,
        header: &header,
        data: None,
    };
    let profile = ApiCall {
        url: PROFILE_URL,
        ..usage
    };
    let (usage, profile) = tokio::join!(management.call(&usage), management.call(&profile));
    let usage = answer(usage?)?;
    let plan = profile
        .ok()
        .and_then(|profile| answer(profile).ok())
        .and_then(|profile| plan(&profile));

    Ok(Observation {
        windows: windows(&usage),
        plan,
    })
}

fn windows(usage: &Value) -> Vec<NewWindow> {
    let fable = fable_limit(usage);
    let mut windows = WINDOWS
        .iter()
        .filter(|(payload_key, ..)| !(*payload_key == FABLE_KEY && fable.is_some()))
        .filter_map(|&(payload_key, key, label)| {
            let window = usage.get(payload_key).filter(|window| window.is_object())?;
            let utilization = window.get("utilization")?;
            Some(NewWindow {
                key: key.to_owned(),
                label: label.to_owned(),
                used_fraction: number(utilization).map(used_from_percent),
                window_seconds: Some(if payload_key == "five_hour" {
                    FIVE_HOURS
                } else {
                    WEEK
                }),
                resets_at: window.get("resets_at").and_then(instant),
                ..NewWindow::default()
            })
        })
        .collect::<Vec<_>>();
    if let Some(limit) = fable {
        windows.push(NewWindow {
            key: "seven-day-fable".to_owned(),
            label: "7-day Fable".to_owned(),
            used_fraction: limit.get("percent").and_then(number).map(used_from_percent),
            window_seconds: Some(WEEK),
            resets_at: limit.get("resets_at").and_then(instant),
            ..NewWindow::default()
        });
    }
    if let Some(extra) = usage
        .get("extra_usage")
        .filter(|extra| extra.get("is_enabled").and_then(Value::as_bool) == Some(true))
    {
        let used = extra.get("used_credits").and_then(number);
        let limit = extra.get("monthly_limit").and_then(number);
        windows.push(NewWindow {
            key: "extra-usage".to_owned(),
            label: "Extra usage".to_owned(),
            used_fraction: extra
                .get("utilization")
                .and_then(number)
                .map(used_from_percent)
                .or_else(|| {
                    used.zip(limit.filter(|limit| *limit > 0.0))
                        .map(|(used, limit)| used / limit)
                }),
            used_value: used,
            limit_value: limit,
            unit: Some("credits"),
            ..NewWindow::default()
        });
    }

    windows
}

/// The weekly limit Claude reports for the Fable model in `limits` rather than as a named
/// window. The active one wins over an inactive one.
fn fable_limit(usage: &Value) -> Option<&Value> {
    let candidates = usage
        .get("limits")?
        .as_array()?
        .iter()
        .filter(|limit| {
            let kind = limit
                .get("kind")
                .and_then(text)
                .map(|kind| kind.to_lowercase());
            let model = limit
                .pointer("/scope/model/display_name")
                .and_then(text)
                .map(|model| model.to_lowercase());
            kind.as_deref() == Some("weekly_scoped")
                && matches!(model.as_deref(), Some("fable" | "fable 5"))
                && limit.get("percent").and_then(number).is_some()
        })
        .collect::<Vec<_>>();

    candidates
        .iter()
        .find(|limit| limit.get("is_active").and_then(Value::as_bool) == Some(true))
        .or_else(|| candidates.first())
        .copied()
}

fn plan(profile: &Value) -> Option<String> {
    let organization = |key: &str| {
        profile
            .pointer(&format!("/organization/{key}"))
            .and_then(text)
            .map(|value| value.to_lowercase())
    };
    if organization("organization_type").as_deref() == Some("claude_team")
        && organization("subscription_status").as_deref() == Some("active")
    {
        return Some("team".to_owned());
    }
    let flag = |key: &str| profile.pointer(&format!("/account/{key}")).and_then(truthy);
    let (max, pro) = (flag("has_claude_max"), flag("has_claude_pro"));
    match (max, pro) {
        (Some(true), _) => Some("max"),
        (_, Some(true)) => Some("pro"),
        (Some(false), Some(false)) => Some("free"),
        _ => None,
    }
    .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_team_token_with_a_personal_max_subscription_is_team() {
        let profile = json!({
            "organization": { "organization_type": "claude_team", "subscription_status": "active" },
            "account": { "has_claude_max": true },
        });

        assert_eq!(plan(&profile).as_deref(), Some("team"));
    }

    #[test]
    fn the_fable_limit_replaces_the_named_window() {
        let usage = json!({
            "iguana_necktie": { "utilization": 10.0, "resets_at": null },
            "limits": [{
                "kind": "weekly_scoped",
                "percent": 64,
                "is_active": true,
                "scope": { "model": { "display_name": "Fable" } },
            }],
        });
        let windows = windows(&usage);

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_fraction, Some(0.64));
    }
}
