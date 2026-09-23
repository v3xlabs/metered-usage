//! Devin: Codeium's `GetUserStatus`, which takes the token in the body rather than a
//! header and reports the percentage remaining.

use jiff::Timestamp;
use serde_json::Value;

use crate::quota::management::{ApiCall, Management};
use crate::quota::provider::{Observation, RefreshError, answer, offset, text};
use crate::quota::window::NewWindow;

const STATUS_URL: &str =
    "https://server.codeium.com/exa.seat_management_pb.SeatManagementService/GetUserStatus";
const STATUS_BODY: &str = r#"{"metadata":{"ideName":"chisel","ideVersion":"3000.10.21","apiKey":"$TOKEN$","locale":"en","os":"darwin","extensionVersion":"3000.10.21","clientName":"chisel"}}"#;
const DAY: i64 = 24 * 60 * 60;

pub async fn fetch(
    management: &Management<'_>,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    let header = [
        ("Content-Type", "application/json".to_owned()),
        ("Connect-Protocol-Version", "1".to_owned()),
    ];
    let status = answer(
        management
            .call(&ApiCall {
                auth_index,
                method: "POST",
                url: STATUS_URL,
                header: &header,
                data: Some(STATUS_BODY),
            })
            .await?,
    )?;
    let plan_status = status
        .pointer("/userStatus/planStatus")
        .unwrap_or(&Value::Null);
    let windows = [("daily", "Daily", DAY), ("weekly", "Weekly", 7 * DAY)]
        .into_iter()
        .filter_map(|(key, label, seconds)| {
            let remaining = plan_status
                .get(format!("{key}QuotaRemainingPercent"))
                .and_then(percent);
            let resets_at = plan_status
                .get(format!("{key}QuotaResetAtUnix"))
                .and_then(unix_seconds);
            (remaining.is_some() || resets_at.is_some()).then(|| NewWindow {
                key: key.to_owned(),
                label: label.to_owned(),
                used_fraction: remaining.map(|remaining| 1.0 - remaining / 100.0),
                window_seconds: Some(seconds),
                resets_at,
                ..NewWindow::default()
            })
        })
        .collect::<Vec<_>>();
    if windows.is_empty() {
        return Err(RefreshError::Empty);
    }

    Ok(Observation {
        windows,
        plan: plan_status.pointer("/planInfo/planName").and_then(text),
    })
}

/// A percentage from 0 to 100, as a number or a plain decimal string.
fn percent(value: &Value) -> Option<f64> {
    let percent = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => {
            let text = text.trim();
            let plain = !text.is_empty()
                && !text.starts_with('.')
                && !text.ends_with('.')
                && text
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
                && text.matches('.').count() <= 1;
            plain.then(|| text.parse().ok()).flatten()
        }
        _ => None,
    }?;
    (0.0..=100.0).contains(&percent).then_some(percent)
}

fn unix_seconds(value: &Value) -> Option<Timestamp> {
    let seconds = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) if text.trim().chars().all(|c| c.is_ascii_digit()) => {
            text.trim().parse().ok()
        }
        _ => None,
    }?;
    (seconds > 0.0)
        .then(|| offset(Timestamp::UNIX_EPOCH, seconds))
        .flatten()
}
