//! Kimi: `coding/v1/usages`, which counts requests used against a limit.

use jiff::Timestamp;
use serde_json::Value;

use crate::quota::management::{ApiCall, Management};
use crate::quota::provider::{
    Observation, RefreshError, after_seconds, answer, field, instant, integer, number, text,
};
use crate::quota::window::{NewWindow, REQUESTS};

const USAGE_URL: &str = "https://api.kimi.com/coding/v1/usages";
const HOUR: i64 = 60 * 60;
const DAY: i64 = 24 * HOUR;

#[derive(Clone, Copy)]
enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
}

pub async fn fetch(
    management: &Management<'_>,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    let header = [("Authorization", "Bearer $TOKEN$".to_owned())];
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

    Ok(Observation {
        windows: windows(&usage, Timestamp::now()),
        plan: None,
    })
}

fn windows(usage: &Value, now: Timestamp) -> Vec<NewWindow> {
    let mut windows = Vec::new();
    let limits = usage
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for (index, item) in limits.enumerate() {
        let detail = item
            .get("detail")
            .filter(|detail| detail.is_object())
            .unwrap_or(item);
        let window = item.get("window").filter(|window| window.is_object());
        let duration = [window, Some(item), Some(detail)]
            .into_iter()
            .flatten()
            .find_map(|record| record.get("duration").and_then(integer));
        let unit = [window, Some(item), Some(detail)]
            .into_iter()
            .flatten()
            .find_map(|record| record.get("timeUnit").and_then(Value::as_str));
        let named = ["name", "title", "scope"].iter().find_map(|key| {
            item.get(key)
                .or_else(|| detail.get(key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        });
        let fallback = named.unwrap_or_else(|| match duration.filter(|duration| *duration > 0) {
            Some(duration) => format!("{} limit", duration_token(duration, unit)),
            None => format!("Limit {}", index + 1),
        });
        if let Some(row) = row(
            detail,
            format!("limit-{index}"),
            fallback,
            duration,
            unit,
            now,
        ) {
            windows.push(row);
        }
    }
    if let Some(summary) = usage.get("usage").filter(|usage| usage.is_object())
        && let Some(row) = row(
            summary,
            "summary".to_owned(),
            "Weekly limit".to_owned(),
            None,
            None,
            now,
        )
    {
        windows.push(row);
    }

    windows
}

fn row(
    data: &Value,
    key: String,
    fallback: String,
    duration: Option<i64>,
    unit: Option<&str>,
    now: Timestamp,
) -> Option<NewWindow> {
    let count = |key: &str| data.get(key).and_then(number).map(f64::floor);
    let limit = count("limit");
    let used = count("used").or_else(|| Some(limit? - count("remaining")?));
    if used.is_none() && limit.is_none() {
        return None;
    }
    let (used, limit) = (used.unwrap_or(0.0), limit.unwrap_or(0.0));
    let label = ["name", "title"]
        .iter()
        .find_map(|key| data.get(key).and_then(text))
        .unwrap_or(fallback);
    let resets_at = field(data, &["reset_at", "resetAt", "reset_time", "resetTime"])
        .and_then(instant)
        .or_else(|| {
            ["reset_in", "resetIn", "ttl"].iter().find_map(|key| {
                data.get(key)
                    .and_then(|seconds| after_seconds(seconds, now))
            })
        });
    let window_seconds = match duration.filter(|duration| *duration > 0) {
        Some(duration) => seconds(duration, unit),
        None => seconds_from_label(&label),
    };

    Some(NewWindow {
        key,
        used_fraction: (limit > 0.0).then(|| used / limit),
        used_value: Some(used),
        limit_value: Some(limit),
        unit: Some(REQUESTS),
        window_seconds,
        resets_at,
        label,
        ..NewWindow::default()
    })
}

/// Kimi sends protobuf names such as `TIME_UNIT_MINUTE`; an absent unit is minutes.
fn time_unit(unit: Option<&str>) -> Option<TimeUnit> {
    let unit = unit.unwrap_or_default().trim().to_uppercase();
    match unit.strip_prefix("TIME_UNIT_").unwrap_or(&unit) {
        "SECOND" | "SECONDS" => Some(TimeUnit::Second),
        "" | "MINUTE" | "MINUTES" => Some(TimeUnit::Minute),
        "HOUR" | "HOURS" => Some(TimeUnit::Hour),
        "DAY" | "DAYS" => Some(TimeUnit::Day),
        "WEEK" | "WEEKS" => Some(TimeUnit::Week),
        _ => None,
    }
}

fn seconds(duration: i64, unit: Option<&str>) -> Option<i64> {
    let factor = match time_unit(unit) {
        Some(TimeUnit::Second) => 1,
        Some(TimeUnit::Minute) | None => 60,
        Some(TimeUnit::Hour) => HOUR,
        Some(TimeUnit::Day) => DAY,
        Some(TimeUnit::Week) => 7 * DAY,
    };
    duration.checked_mul(factor)
}

fn duration_token(duration: i64, unit: Option<&str>) -> String {
    match time_unit(unit) {
        Some(TimeUnit::Second) => format!("{duration}s"),
        Some(TimeUnit::Hour) => format!("{duration}h"),
        Some(TimeUnit::Day) => format!("{duration}d"),
        Some(TimeUnit::Week) => format!("{duration}w"),
        Some(TimeUnit::Minute) | None if duration % 60 == 0 => format!("{}h", duration / 60),
        Some(TimeUnit::Minute) | None => format!("{duration}m"),
    }
}

fn seconds_from_label(label: &str) -> Option<i64> {
    let label = label.to_lowercase();
    if label.contains("day") {
        Some(DAY)
    } else if label.contains("week") {
        Some(7 * DAY)
    } else if label.contains("month") {
        Some(30 * DAY)
    } else if label.contains("5h") || label.contains("hour") {
        Some(5 * HOUR)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_limit_given_as_remaining_reads_as_used() {
        let usage = json!({
            "limits": [{
                "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                "detail": { "limit": "200", "remaining": "150" },
            }],
        });
        let windows = windows(&usage, Timestamp::UNIX_EPOCH);

        assert_eq!(windows[0].used_fraction, Some(0.25));
        assert_eq!(windows[0].window_seconds, Some(5 * HOUR));
        assert_eq!(windows[0].label, "5h limit");
    }
}
