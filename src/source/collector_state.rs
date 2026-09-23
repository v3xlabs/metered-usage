//! What the resident collector for a source last reported about itself.

use jiff::Timestamp;

#[derive(Debug, Clone)]
pub struct CollectorState {
    pub connected_since: Option<Timestamp>,
    pub last_record_at: Option<Timestamp>,
    pub records_received: i64,
    pub consecutive_failures: i64,
    pub last_error: Option<String>,
    pub updated_at: Timestamp,
}
