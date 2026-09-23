//! Polls LiteLLM's `/spend/logs/v2` for the rows written since the last poll.
//!
//! The endpoint has no cursor, only a time window and offset pages, and a row is written
//! when its request ends, so it can land behind a later `startTime`. Each poll reaches
//! back past the newest stored event and lets the record hash drop what it already has.

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;
use serde_json::Value;

use crate::app::AppState;
use crate::collector::{Backoff, CollectorError, Progress};
use crate::config::SourceConfig;
use crate::id::Id;
use crate::source::Source;
use crate::usage::ingest::{IngestReport, ingest};
use crate::usage::litellm;

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const PAGE_SIZE: usize = 500;
const OVERLAP: SignedDuration = SignedDuration::from_mins(15);
const FIRST_WINDOW: SignedDuration = SignedDuration::from_hours(30 * 24);
/// LiteLLM requires an end to the window; a little past now keeps a proxy whose clock
/// runs ahead from hiding its newest rows.
const CLOCK_SLACK: SignedDuration = SignedDuration::from_mins(5);
/// The only window format LiteLLM parses with a time of day, read as UTC.
const WINDOW_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

#[derive(Debug, Deserialize)]
struct Page {
    data: Vec<Value>,
}

pub async fn run(state: &AppState, source_id: Id<Source>, config: &SourceConfig) {
    let progress = Progress { source_id };
    let mut backoff = Backoff::default();
    let client = match reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(source = config.key, %error, "no HTTP client, not collecting");
            return;
        }
    };
    loop {
        match poll(state, &client, source_id, config).await {
            Ok(_) => {
                backoff.reset();
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            Err(error) => {
                progress
                    .retry_after(&state.database, &config.key, &error, &mut backoff)
                    .await;
            }
        }
    }
}

/// One poll over the window from just before the newest stored event to now, every
/// page of it, each page committed as it arrives.
pub async fn poll(
    state: &AppState,
    client: &reqwest::Client,
    source_id: Id<Source>,
    config: &SourceConfig,
) -> Result<IngestReport, CollectorError> {
    let progress = Progress { source_id };
    let now = Timestamp::now();
    let newest = Source::load(&state.database, source_id)
        .await?
        .and_then(|source| source.last_event_at)
        .map(Timestamp::from);
    let start = newest.map_or(now - FIRST_WINDOW, |newest| newest - OVERLAP);
    let start_date = start.strftime(WINDOW_FORMAT).to_string();
    let end_date = (now + CLOCK_SLACK).strftime(WINDOW_FORMAT).to_string();
    let endpoint = format!("{}/spend/logs/v2", config.base_url);
    let page_size = PAGE_SIZE.to_string();
    let mut total = IngestReport::default();
    for page in 1.. {
        let url = reqwest::Url::parse_with_params(
            &endpoint,
            [
                ("start_date", start_date.as_str()),
                ("end_date", end_date.as_str()),
                ("sort_by", "startTime"),
                ("sort_order", "asc"),
                ("page", &page.to_string()),
                ("page_size", &page_size),
            ],
        )
        .map_err(|_| CollectorError::Address(config.base_url.clone()))?;
        let response = client
            .get(url)
            .bearer_auth(config.secret.expose())
            .send()
            .await?;
        // The body of a refusal can quote the key it refused, so only the status is kept.
        if !response.status().is_success() {
            return Err(CollectorError::Status(response.status()));
        }
        let rows = response.json::<Page>().await?.data;
        let short = rows.len() < PAGE_SIZE;
        if !rows.is_empty() {
            let report = ingest(
                state,
                source_id,
                rows.into_iter().map(litellm::parse).collect(),
            )
            .await?;
            progress.received(&state.database, &report).await?;
            total.accepted += report.accepted;
            total.duplicates += report.duplicates;
            total.rejected += report.rejected;
        }
        if short {
            break;
        }
    }
    progress.polled(&state.database).await?;
    Ok(total)
}
