//! Claude: `/api/oauth/usage` for the windows, `/api/oauth/profile` for the plan and the
//! usage again with `cedar_ember=1` for the manual reset grants. Only the first answer is
//! required.

use std::collections::HashSet;

use serde_json::Value;

use crate::quota::management::{ApiCall, Management};
use crate::quota::manual_reset::Grant;
use crate::quota::provider::{
    ManualResets, Observation, RefreshError, answer, instant, number, text, truthy,
    used_from_percent,
};
use crate::quota::window::{NewWindow, Scope};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const RESET_GRANTS_URL: &str =
    "https://api.anthropic.com/api/oauth/usage?cedar_ember=1&skip_spend=1";
/// The UI reads the reset grants as Claude Code, whose version decides eligibility.
const USER_AGENT: &str = "claude-cli/2.1.280 (external, cli)";
/// The UI shows at most this many characters of a grant's label.
const LABEL_CHARACTERS: usize = 120;
const FIVE_HOURS: i64 = 5 * 60 * 60;
const WEEK: i64 = 7 * 24 * 60 * 60;
const FABLE_KEY: &str = "iguana_necktie";
/// Organization types that are a plan of their own. Personal subscriptions name an
/// organization type too, but only the account flags say which of them a token holds.
const ORGANIZATION_PLANS: [&str; 2] = ["team", "enterprise"];

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
    let grants_header = [
        ("User-Agent", USER_AGENT.to_owned()),
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("Content-Type", "application/json".to_owned()),
        ("anthropic-beta", "oauth-2025-04-20".to_owned()),
    ];
    let usage_call = ApiCall {
        auth_index,
        method: "GET",
        url: USAGE_URL,
        header: &header,
        data: None,
    };
    let profile = ApiCall {
        url: PROFILE_URL,
        ..usage_call
    };
    let (usage, profile) = tokio::join!(management.call(&usage_call), management.call(&profile));
    let usage = answer(usage?)?;
    let plan = profile
        .ok()
        .and_then(|profile| answer(profile).ok())
        .and_then(|profile| plan(&profile));
    // Read only once the usage answered, as Codex reads its list, so the extra request never
    // competes with the one the refresh needs.
    let grants = ApiCall {
        url: RESET_GRANTS_URL,
        header: &grants_header,
        ..usage_call
    };
    let manual_resets = management
        .call(&grants)
        .await
        .map_err(RefreshError::from)
        .and_then(answer)
        .and_then(|body| reset_grants(&body).ok_or(RefreshError::Unreadable))
        .inspect_err(|error| tracing::warn!(%error, "the Claude reset grants were not read"))
        .map_or(ManualResets::Unchanged, ManualResets::Reported);

    Ok(Observation {
        windows: windows(&usage),
        plan,
        manual_resets,
    })
}

/// The grants with a reset left. A block without a boolean `eligible`, or with one grant
/// without an id, a repeated id, a bad count or a bad end, is not read at all, so it never
/// clears the stored grants.
fn reset_grants(body: &Value) -> Option<Vec<Grant>> {
    let block = body.get("cedar_ember")?;
    let eligible = block.get("eligible")?.as_bool()?;
    let reason = block.get("ineligible_reason").and_then(Value::as_str);
    if !eligible && reason == Some("cli_version") {
        tracing::warn!(
            user_agent = USER_AGENT,
            "Claude refused the reset grants for this client version; the stored grants are cleared"
        );
    }
    let listed = match block.get("grants") {
        None | Some(Value::Null) => return Some(Vec::new()),
        Some(listed) => listed.as_array()?,
    };
    let mut ids = HashSet::with_capacity(listed.len());
    let mut grants = Vec::with_capacity(listed.len());
    for value in listed {
        let id = value.get("id")?.as_str().filter(|id| !id.is_empty())?;
        if !ids.insert(id) {
            return None;
        }
        grants.push(grant(value)?);
    }
    grants.retain(|grant| grant.remaining > 0);

    Some(grants)
}

fn grant(value: &Value) -> Option<Grant> {
    let total = value
        .get("resets_total")?
        .as_i64()
        .filter(|total| *total >= 0)?;
    let remaining = value
        .get("resets_left")?
        .as_i64()
        .filter(|left| (0..=total).contains(left))?;
    let expires_at = match value.get("ends_at") {
        None | Some(Value::Null) => None,
        Some(end) => Some(instant(end)?.into()),
    };

    Some(Grant {
        label: value.get("label").and_then(Value::as_str).and_then(label),
        remaining,
        total: Some(total),
        expires_at,
    })
}

/// Provider text shown as is, so control characters and runs of whitespace become one space.
fn label(text: &str) -> Option<String> {
    let mut label = String::new();
    for word in text
        .split(|character: char| character.is_whitespace() || character.is_control())
        .filter(|word| !word.is_empty())
    {
        if !label.is_empty() {
            label.push(' ');
        }
        label.push_str(word);
    }
    if let Some((end, _)) = label.char_indices().nth(LABEL_CHARACTERS) {
        label.truncate(end);
    }

    (!label.is_empty()).then_some(label)
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
                scope: scope(key),
                starts_on_use: payload_key == "five_hour",
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
            scope: scope("seven-day-fable"),
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

/// The model limits count only their own model. Cowork counts work Claude runs itself,
/// which never passes a gateway.
fn scope(key: &str) -> Scope {
    match key {
        "seven-day-opus" => Scope::Model("opus".to_owned()),
        "seven-day-sonnet" => Scope::Model("sonnet".to_owned()),
        "seven-day-fable" => Scope::Model("fable".to_owned()),
        "seven-day-cowork" => Scope::Unmetered,
        _ => Scope::Account,
    }
}

/// An active team or enterprise organization wins over the account flags: they describe the
/// person's own subscription, and read as free for a seat.
fn plan(profile: &Value) -> Option<String> {
    let organization = |key: &str| {
        profile
            .pointer(&format!("/organization/{key}"))
            .and_then(text)
            .map(|value| value.to_lowercase())
    };
    if organization("subscription_status").as_deref() == Some("active")
        && let Some(kind) = organization("organization_type")
            .as_deref()
            .and_then(|kind| kind.strip_prefix("claude_"))
            .filter(|kind| ORGANIZATION_PLANS.contains(kind))
    {
        return Some(kind.to_owned());
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
    fn an_enterprise_seat_is_enterprise_and_a_personal_plan_follows_the_flags() {
        let seat = json!({
            "organization": { "organization_type": "claude_enterprise", "subscription_status": "active" },
            "account": { "has_claude_max": false, "has_claude_pro": false },
        });
        let personal = json!({
            "organization": { "organization_type": "claude_max", "subscription_status": "active" },
            "account": { "has_claude_max": false, "has_claude_pro": true },
        });

        assert_eq!(plan(&seat).as_deref(), Some("enterprise"));
        assert_eq!(plan(&personal).as_deref(), Some("pro"));
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

    #[test]
    fn the_recorded_grants_list_those_with_a_reset_left() {
        let body = serde_json::from_str::<Value>(include_str!(
            "../../../tests/fixtures/claude_reset_grants.json"
        ))
        .expect("the fixture is JSON");

        assert_eq!(
            reset_grants(&body),
            Some(vec![
                Grant {
                    label: Some(
                        "Claude Opus 5.5 launch: one usage-limit reset for Pro and Max".to_owned()
                    ),
                    remaining: 1,
                    total: Some(1),
                    expires_at: Some(
                        "2026-10-22T16:00:00Z"
                            .parse::<jiff::Timestamp>()
                            .expect("an instant")
                            .into()
                    ),
                },
                Grant {
                    label: Some("Max loyalty resets".to_owned()),
                    remaining: 2,
                    total: Some(3),
                    expires_at: None,
                },
            ])
        );
    }

    #[test]
    fn a_malformed_block_is_not_read_at_all() {
        let block = |second: Value| {
            json!({ "cedar_ember": { "eligible": true, "grants": [
                { "id": "a", "resets_total": 1, "resets_left": 1, "ends_at": null },
                second,
            ] } })
        };
        let used_up = json!({ "id": "b", "resets_total": 1, "resets_left": 0 });

        for second in [
            json!({ "id": "b", "resets_total": 1, "resets_left": 2 }),
            json!({ "id": "b", "resets_total": 1, "resets_left": "1" }),
            json!({ "id": "b", "resets_total": 1, "resets_left": 1, "ends_at": "soon" }),
            json!({ "resets_total": 1, "resets_left": 1 }),
            json!({ "id": "", "resets_total": 1, "resets_left": 1 }),
            json!({ "id": "a", "resets_total": 1, "resets_left": 1 }),
        ] {
            assert_eq!(reset_grants(&block(second.clone())), None, "{second}");
        }
        assert_eq!(reset_grants(&json!({ "five_hour": {} })), None);
        assert_eq!(
            reset_grants(&json!({ "cedar_ember": { "grants": [used_up.clone()] } })),
            None
        );

        let read = reset_grants(&block(used_up)).expect("a readable block");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].expires_at, None);
        assert_eq!(read[0].label, None);
    }

    #[test]
    fn a_label_is_one_line_of_at_most_120_characters() {
        assert_eq!(
            label(" Opus\u{0}launch\n\treset ").as_deref(),
            Some("Opus launch reset")
        );
        assert_eq!(label("é".repeat(130).as_str()), Some("é".repeat(120)));
        assert_eq!(label(" \n "), None);
    }
}
