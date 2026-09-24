//! xAI: a free credential reads Grok CLI's two billing endpoints; a paid one, or a free one
//! whose billing says nothing, is checked through `api.x.ai` with a one-token chat
//! completion, which spends real quota and reports no numbers. The choice between the two
//! is the UI's.

use jiff::Timestamp;
use serde_json::Value;

use crate::quota::management::{ApiCall, AuthFile, Management};
use crate::quota::provider::{
    Observation, RefreshError, answer, field, instant, number, slug, text, truthy,
    used_from_percent,
};
use crate::quota::window::NewWindow;

const BILLING_WEEKLY_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const BILLING_MONTHLY_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing";
const ME_URL: &str = "https://api.x.ai/v1/me";
const CHAT_URL: &str = "https://api.x.ai/v1/chat/completions";
const PING_BODY: &str = r#"{"model":"grok-4.5","messages":[{"role":"user","content":"ping"}],"max_tokens":1,"stream":false}"#;
const CLIENT_VERSION: &str = "0.2.91";
const USER_AGENT: &str = "grok-pager/0.2.91 grok-shell/0.2.91 (macos; aarch64)";
const PAID_PREFIX: &str = "paid";
const NESTED_AUTH_KEYS: [&str; 6] = [
    "metadata",
    "attributes",
    "oauth",
    "raw",
    "credential",
    "auth",
];
const NESTED_DEPTH: usize = 2;
const CENTS: &str = "usd_cents";

pub async fn fetch(
    management: &Management<'_>,
    file: &AuthFile,
    auth_index: &str,
) -> Result<Observation, RefreshError> {
    if is_paid(&file.raw) {
        return paid(management, auth_index).await;
    }
    let mut header = vec![
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("x-xai-token-auth", "xai-grok-cli".to_owned()),
        ("x-grok-client-version", CLIENT_VERSION.to_owned()),
        ("accept", "*/*".to_owned()),
        ("user-agent", USER_AGENT.to_owned()),
    ];
    if let Some(user_id) = user_id(&file.raw) {
        header.push(("x-userid", user_id));
    }
    let billing = |url| {
        let header = &header;
        async move {
            let payload = answer(
                management
                    .call(&ApiCall {
                        auth_index,
                        method: "GET",
                        url,
                        header,
                        data: None,
                    })
                    .await?,
            )?;
            Ok::<_, RefreshError>(payload.get("config").and_then(billing_windows))
        }
    };
    let (weekly, monthly) = tokio::join!(billing(BILLING_WEEKLY_URL), billing(BILLING_MONTHLY_URL));
    // When both billing calls fail the first one's error is what is reported, as the UI
    // does, even though the paid check runs after it.
    let failure = match (weekly, monthly) {
        (Err(error), Err(_)) => error,
        (weekly, monthly) => {
            let windows = [weekly, monthly]
                .into_iter()
                .filter_map(|summary| summary.ok().flatten())
                .flatten()
                .collect::<Vec<_>>();
            if !windows.is_empty() {
                return Ok(Observation {
                    windows,
                    plan: None,
                });
            }
            RefreshError::Empty
        }
    };
    paid(management, auth_index).await.map_err(|_| failure)
}

async fn paid(management: &Management<'_>, auth_index: &str) -> Result<Observation, RefreshError> {
    let profile_header = [
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("accept", "application/json".to_owned()),
    ];
    let chat_header = [
        ("Authorization", "Bearer $TOKEN$".to_owned()),
        ("accept", "application/json".to_owned()),
        ("Content-Type", "application/json".to_owned()),
    ];
    let profile = ApiCall {
        auth_index,
        method: "GET",
        url: ME_URL,
        header: &profile_header,
        data: None,
    };
    let chat = ApiCall {
        auth_index,
        method: "POST",
        url: CHAT_URL,
        header: &chat_header,
        data: Some(PING_BODY),
    };
    let (_, chat) = tokio::join!(management.call(&profile), management.call(&chat));
    let chat = chat?;
    if !(200..300).contains(&chat.status_code) {
        return Err(RefreshError::Upstream(chat.status_code));
    }

    Ok(Observation {
        windows: Vec::new(),
        plan: Some(PAID_PREFIX.to_owned()),
    })
}

/// Paid when the credential routes through the official API under the `paid` prefix, or
/// when any token it carries states a tier of at least one.
fn is_paid(entry: &Value) -> bool {
    let mut records = Vec::new();
    collect(entry, 0, &mut records);
    let official = records.iter().any(|record| {
        field(record, &["using_api", "usingApi"])
            .and_then(truthy)
            .unwrap_or(false)
    });
    let prefixed = records.iter().any(|record| {
        record
            .get("prefix")
            .and_then(text)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PAID_PREFIX))
    });
    if official && prefixed {
        return true;
    }
    records.iter().any(|record| {
        [
            "access_token",
            "accessToken",
            "id_token",
            "idToken",
            "token",
        ]
        .iter()
        .filter_map(|key| record.get(key).and_then(Value::as_str))
        .any(|token| tier(token).is_some_and(|tier| tier >= 1.0))
    })
}

fn collect<'a>(value: &'a Value, depth: usize, records: &mut Vec<&'a Value>) {
    if !value.is_object() || depth > NESTED_DEPTH {
        return;
    }
    records.push(value);
    for key in NESTED_AUTH_KEYS {
        if let Some(nested) = value.get(key) {
            collect(nested, depth + 1, records);
        }
    }
}

/// The `tier` claim of a JWT, under any namespace.
fn tier(token: &str) -> Option<f64> {
    let payload = token.split('.').nth(1)?;
    let claims = serde_json::from_slice::<Value>(&base64_url(payload)?).ok()?;
    claims
        .as_object()?
        .iter()
        .find(|(key, _)| {
            let key = key.to_lowercase();
            key == "tier" || key.ends_with("/tier") || key.ends_with(":tier")
        })
        .and_then(|(_, tier)| number(tier))
}

fn base64_url(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0;
    for character in text.trim_end_matches('=').bytes() {
        let sextet = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }

    Some(bytes)
}

fn user_id(entry: &Value) -> Option<String> {
    let metadata = entry.get("metadata");
    let attributes = entry.get("attributes");
    let nested = |key: &str| {
        entry
            .get(key)
            .or_else(|| metadata.and_then(|metadata| metadata.get(key)))
            .or_else(|| attributes.and_then(|attributes| attributes.get(key)))
            .filter(|value| value.is_object())
    };
    let direct = [Some(entry), metadata, attributes]
        .into_iter()
        .flatten()
        .flat_map(|record| {
            ["sub", "subject", "user_id", "userId"]
                .into_iter()
                .filter_map(|key| record.get(key))
        });
    let oauth = nested("oauth").into_iter().flat_map(|oauth| {
        ["sub", "subject"]
            .into_iter()
            .filter_map(|key| oauth.get(key))
    });
    let user = nested("user")
        .into_iter()
        .flat_map(|user| ["sub", "id"].into_iter().filter_map(|key| user.get(key)));

    direct.chain(oauth).chain(user).find_map(text)
}

/// The windows one billing answer states. A weekly credit percentage is a quota window;
/// the monthly figures are a spend cap that rolls over with the billing period.
fn billing_windows(config: &Value) -> Option<Vec<NewWindow>> {
    let period = field(config, &["currentPeriod", "current_period"]);
    let period_type = period
        .and_then(|period| period.get("type"))
        .and_then(text)
        .map(|kind| kind.to_lowercase())
        .unwrap_or_default();
    let credit_percent =
        field(config, &["creditUsagePercent", "credit_usage_percent"]).and_then(number);
    let period_start = period
        .and_then(|period| period.get("start"))
        .and_then(instant)
        .or_else(|| {
            field(config, &["billingPeriodStart", "billing_period_start"]).and_then(instant)
        });
    let period_end = period
        .and_then(|period| period.get("end"))
        .and_then(instant)
        .or_else(|| field(config, &["billingPeriodEnd", "billing_period_end"]).and_then(instant));
    let products = field(config, &["productUsage", "product_usage"])
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let cents = |keys: &[&str]| {
        field(config, keys).and_then(|value| number(value.get("val").unwrap_or(value)))
    };
    let monthly_limit = cents(&["monthlyLimit", "monthly_limit"]);
    let used = cents(&["used"]);
    let on_demand_cap = cents(&["onDemandCap", "on_demand_cap"]);
    let on_demand_used = cents(&["onDemandUsed", "on_demand_used"]);
    let billing_start =
        field(config, &["billingPeriodStart", "billing_period_start"]).and_then(instant);
    let billing_end = field(config, &["billingPeriodEnd", "billing_period_end"]).and_then(instant);

    let weekly = credit_percent.is_some() || period_type.contains("weekly") || !products.is_empty();
    let monthly = monthly_limit.is_some()
        || used.is_some()
        || (!weekly && (on_demand_cap.is_some() || billing_end.is_some()));
    if !weekly && !monthly {
        return None;
    }

    let mut windows = if weekly {
        credit_windows(
            &period_type,
            credit_percent,
            products,
            period_start,
            period_end,
        )
    } else {
        Vec::new()
    };
    if monthly {
        let included = used.map(|used| match monthly_limit {
            Some(limit) if limit > 0.0 => used.min(limit),
            _ => used,
        });
        windows.push(NewWindow {
            key: "included-spend".to_owned(),
            label: "Included spend".to_owned(),
            used_fraction: included
                .zip(monthly_limit.filter(|limit| *limit > 0.0))
                .map(|(used, limit)| used / limit),
            used_value: included,
            limit_value: monthly_limit,
            unit: Some(CENTS),
            window_seconds: span(billing_start, billing_end),
            resets_at: billing_end,
            ..NewWindow::default()
        });
    }
    if let Some(cap) = on_demand_cap {
        let on_demand = on_demand_used.or_else(|| {
            used.zip(monthly_limit)
                .map(|(used, limit)| (used - limit).max(0.0))
        });
        windows.push(NewWindow {
            key: "on-demand-spend".to_owned(),
            label: "On-demand spend".to_owned(),
            used_fraction: on_demand
                .filter(|_| cap > 0.0)
                .map(|on_demand| on_demand / cap),
            used_value: on_demand,
            limit_value: Some(cap),
            unit: Some(CENTS),
            window_seconds: span(billing_start, billing_end),
            resets_at: billing_end,
            ..NewWindow::default()
        });
    }

    Some(windows)
}

/// The credit window of the active period and one window per product within it.
fn credit_windows(
    period_type: &str,
    credit_percent: Option<f64>,
    products: &[Value],
    start: Option<Timestamp>,
    end: Option<Timestamp>,
) -> Vec<NewWindow> {
    let label = if period_type.contains("monthly") {
        "Monthly credits"
    } else {
        "Weekly credits"
    };
    let credits = NewWindow {
        key: "credits".to_owned(),
        label: label.to_owned(),
        used_fraction: credit_percent.map(used_from_percent),
        window_seconds: span(start, end),
        resets_at: end,
        ..NewWindow::default()
    };
    let products = products.iter().enumerate().map(|(index, product)| {
        let name = product
            .get("product")
            .and_then(text)
            .unwrap_or_else(|| format!("Product {}", index + 1));
        NewWindow {
            key: format!("product-{}", slug(&name)),
            label: name,
            used_fraction: field(product, &["usagePercent", "usage_percent"])
                .and_then(number)
                .map(used_from_percent),
            window_seconds: span(start, end),
            resets_at: end,
            ..NewWindow::default()
        }
    });

    std::iter::once(credits).chain(products).collect()
}

fn span(start: Option<Timestamp>, end: Option<Timestamp>) -> Option<i64> {
    let (start, end) = (start?, end?);
    (end > start).then(|| end.as_second() - start.as_second())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_token_with_a_tier_marks_the_credential_paid() {
        // {"https://x.ai/tier":2}
        let token = "e30.eyJodHRwczovL3guYWkvdGllciI6Mn0.sig";

        assert!(is_paid(&json!({ "metadata": { "access_token": token } })));
        assert!(!is_paid(&json!({ "prefix": "paid" })));
    }

    #[test]
    fn monthly_spend_is_capped_at_the_included_limit() {
        let config = json!({
            "monthlyLimit": { "val": 1000 },
            "used": { "val": 1500 },
            "billingPeriodStart": "2026-09-01T00:00:00Z",
            "billingPeriodEnd": "2026-10-01T00:00:00Z",
        });
        let windows = billing_windows(&config).expect("monthly figures");

        assert_eq!(windows[0].used_fraction, Some(1.0));
        assert_eq!(windows[0].used_value, Some(1000.0));
        assert_eq!(windows[0].window_seconds, Some(30 * 86_400));
    }
}
