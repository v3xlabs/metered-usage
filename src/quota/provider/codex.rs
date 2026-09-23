//! Codex: `wham/usage`. The UI's other calls fetch the subscription end date and the count
//! of rate-limit reset credits, which have no place in a quota window, so they are not
//! sent. The credit-consuming reset is never sent.

use jiff::Timestamp;
use serde_json::Value;

use crate::quota::management::{ApiCall, AuthFile, Management};
use crate::quota::provider::{
    Observation, RefreshError, after_seconds, answer, entry_records, field, instant, integer,
    number, slug, text, used_from_percent,
};
use crate::quota::window::NewWindow;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const USER_AGENT: &str =
    "codex-tui/0.149.1 (Mac OS 26.5.2; arm64) iTerm.app/3.6.11 (codex-tui; 0.149.1)";
const FIVE_HOURS: i64 = 18_000;
const WEEK: i64 = 604_800;
const SHORTEST_MONTH: i64 = 28 * 86_400;
const LONGEST_MONTH: i64 = 31 * 86_400;

pub async fn fetch(
    management: &Management<'_>,
    file: &AuthFile,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    let mut header = vec![
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("Content-Type", "application/json".to_owned()),
        ("User-Agent", USER_AGENT.to_owned()),
    ];
    if let Some(account_id) = chatgpt_account_id(file) {
        header.push(("Chatgpt-Account-Id", account_id));
    }
    let usage = answer(
        management
            .call(&ApiCall {
                auth_index,
                method: "GET",
                url: USAGE_URL,
                header: &header,
                data: None,
            })
            .await?,
    )?;
    let plan = field(&usage, &["plan_type", "planType"])
        .and_then(text)
        .map(|plan| plan.to_lowercase())
        .or_else(|| {
            file.entry
                .id_token
                .as_ref()
                .and_then(|claims| claims.get("plan_type"))
                .and_then(text)
                .map(|plan| plan.to_lowercase())
        });

    Ok(Observation {
        windows: windows(&usage, Timestamp::now()),
        plan,
    })
}

fn chatgpt_account_id(file: &AuthFile) -> Option<String> {
    let records = entry_records(file);
    records
        .iter()
        .flatten()
        .find_map(|record| field(record, &["chatgpt_account_id", "chatgptAccountId"]))
        .and_then(text)
        .or_else(|| {
            records
                .iter()
                .flatten()
                .filter_map(|record| record.get("id_token"))
                .filter(|claims| claims.is_object())
                .find_map(|claims| field(claims, &["chatgpt_account_id", "chatgptAccountId"]))
                .and_then(text)
        })
}

fn windows(usage: &Value, now: Timestamp) -> Vec<NewWindow> {
    let mut windows = Vec::new();
    if let Some(limit) = field(usage, &["rate_limit", "rateLimit"]) {
        push_pair(&mut windows, limit, "", "", now);
    }
    if let Some(limit) = field(usage, &["code_review_rate_limit", "codeReviewRateLimit"]) {
        push_pair(&mut windows, limit, "code-review-", "Code review ", now);
    }
    let additional = field(usage, &["additional_rate_limits", "additionalRateLimits"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for (index, item) in additional.enumerate() {
        let Some(limit) = field(item, &["rate_limit", "rateLimit"]) else {
            continue;
        };
        let fallback = format!("additional-{}", index + 1);
        let name = field(item, &["limit_name", "limitName"])
            .and_then(text)
            .or_else(|| field(item, &["metered_feature", "meteredFeature"]).and_then(text))
            .unwrap_or_else(|| fallback.clone());
        let prefix = Some(slug(&name))
            .filter(|prefix| !prefix.is_empty())
            .unwrap_or(fallback);
        let (short, long) = classify(limit);
        let reached = reached(limit);
        if let Some(window) = short {
            windows.push(read(
                window,
                format!("{prefix}-five-hour-{index}"),
                format!("{name} 5-hour"),
                reached,
                now,
            ));
        }
        if let Some(window) = long {
            let (id, label) = if is_monthly(window) {
                ("monthly", "monthly")
            } else {
                ("weekly", "weekly")
            };
            windows.push(read(
                window,
                format!("{prefix}-{id}-{index}"),
                format!("{name} {label}"),
                reached,
                now,
            ));
        }
    }

    windows
}

fn push_pair(
    windows: &mut Vec<NewWindow>,
    limit: &Value,
    key_prefix: &str,
    label_prefix: &str,
    now: Timestamp,
) {
    let (short, long) = classify(limit);
    let reached = reached(limit);
    if let Some(window) = short {
        windows.push(read(
            window,
            format!("{key_prefix}five-hour"),
            format!("{label_prefix}5-hour"),
            reached,
            now,
        ));
    }
    if let Some(window) = long {
        let (id, label) = if is_monthly(window) {
            ("monthly", "Monthly")
        } else {
            ("weekly", "Weekly")
        };
        let label = if label_prefix.is_empty() {
            label.to_owned()
        } else {
            format!("{label_prefix}{}", label.to_lowercase())
        };
        windows.push(read(
            window,
            format!("{key_prefix}{id}"),
            label,
            reached,
            now,
        ));
    }
}

/// Sorts the primary and secondary window into the five-hour and the weekly or monthly
/// one by their stated length, falling back to their order when no length is stated.
fn classify(limit: &Value) -> (Option<&Value>, Option<&Value>) {
    let primary = field(limit, &["primary_window", "primaryWindow"]);
    let secondary = field(limit, &["secondary_window", "secondaryWindow"]);
    let mut short = None;
    let mut long = None;
    for window in [primary, secondary].into_iter().flatten() {
        match seconds(window) {
            Some(seconds) if seconds == FIVE_HOURS && short.is_none() => short = Some(window),
            Some(seconds) if (seconds == WEEK || is_monthly(window)) && long.is_none() => {
                long = Some(window);
            }
            _ => {}
        }
    }
    let same = |left: Option<&Value>, right: Option<&Value>| {
        left.zip(right)
            .is_some_and(|(left, right)| std::ptr::eq(left, right))
    };
    if short.is_none() && !same(primary, long) {
        short = primary;
    }
    if long.is_none() && !same(secondary, short) {
        long = secondary;
    }

    (short, long)
}

fn seconds(window: &Value) -> Option<i64> {
    field(window, &["limit_window_seconds", "limitWindowSeconds"]).and_then(integer)
}

fn is_monthly(window: &Value) -> bool {
    seconds(window).is_some_and(|seconds| (SHORTEST_MONTH..=LONGEST_MONTH).contains(&seconds))
}

fn reached(limit: &Value) -> bool {
    field(limit, &["limit_reached", "limitReached"]).and_then(Value::as_bool) == Some(true)
        || limit.get("allowed").and_then(Value::as_bool) == Some(false)
}

fn read(window: &Value, key: String, label: String, reached: bool, now: Timestamp) -> NewWindow {
    let resets_at = field(window, &["reset_at", "resetAt"])
        .and_then(instant)
        .or_else(|| {
            field(window, &["reset_after_seconds", "resetAfterSeconds"])
                .and_then(|seconds| after_seconds(seconds, now))
        });
    let used = field(window, &["used_percent", "usedPercent"])
        .and_then(number)
        .map(used_from_percent)
        .or_else(|| (reached && resets_at.is_some()).then_some(1.0));

    NewWindow {
        key,
        label,
        used_fraction: used,
        window_seconds: seconds(window).filter(|seconds| *seconds > 0),
        resets_at,
        ..NewWindow::default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_monthly_secondary_window_is_named_monthly() {
        let usage = json!({
            "rate_limit": {
                "primary_window": { "used_percent": 12, "limit_window_seconds": 18000 },
                "secondary_window": { "used_percent": 40, "limit_window_seconds": 2_592_000 },
            },
        });
        let windows = windows(&usage, Timestamp::UNIX_EPOCH);
        let keys = windows
            .iter()
            .map(|window| window.key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(keys, ["five-hour", "monthly"]);
        assert_eq!(windows[1].window_seconds, Some(2_592_000));
    }

    #[test]
    fn a_reached_limit_without_a_percentage_reads_as_used_up() {
        let usage = json!({
            "rate_limit": {
                "limit_reached": true,
                "primary_window": { "reset_after_seconds": 60 },
            },
        });
        let windows = windows(&usage, Timestamp::UNIX_EPOCH);

        assert_eq!(windows[0].used_fraction, Some(1.0));
    }
}
