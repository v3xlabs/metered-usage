//! Stretches of the viewer's local time that a series is cut into.

use jiff::civil::{Date, DateTime};
use jiff::tz::Offset;
use jiff::{Timestamp, ToSpan};
use sqlx::{QueryBuilder, Sqlite};

use crate::analytics::filter::AnalyticsFilter;
use crate::analytics::{AnalyticsError, MAX_BUCKETS};

/// A week starts on Monday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Hour,
    Day,
    Week,
}

/// One bucket of a window: its key as SQL spells it and the instant it starts.
#[derive(Debug, Clone)]
pub struct BucketStart {
    pub key: String,
    pub start: Timestamp,
}

impl Bucket {
    /// The key of the bucket an event falls in, in the viewer's local time. The stored
    /// instant is cut to whole seconds first, because SQLite rounds a fraction to the
    /// millisecond and would move the last instant of a bucket into the next one.
    pub fn push_expression(self, builder: &mut QueryBuilder<Sqlite>, filter: &AnalyticsFilter) {
        let (head, tail) = match self {
            Self::Hour => ("strftime('%Y-%m-%dT%H', ", ")"),
            Self::Day => ("date(", ")"),
            Self::Week => ("date(", ", 'weekday 0', '-6 days')"),
        };
        builder
            .push(head)
            .push("substr(usage_event.occurred_at, 1, 19), ")
            .push_bind(filter.shift())
            .push(tail);
    }

    /// Every bucket that overlaps `[from, to)`, in order, keyed as
    /// [`Self::push_expression`] keys them.
    pub fn starts(
        self,
        from: Timestamp,
        to: Timestamp,
        offset: Offset,
    ) -> Result<Vec<BucketStart>, AnalyticsError> {
        let mut local = self.floor(offset.to_datetime(from));
        let mut starts = Vec::new();
        loop {
            let start = offset.to_timestamp(local)?;
            if start >= to {
                return Ok(starts);
            }
            if starts.len() == MAX_BUCKETS {
                return Err(AnalyticsError::TooManyBuckets);
            }
            starts.push(BucketStart {
                key: self.key(local),
                start,
            });
            local = match self {
                Self::Hour => local.checked_add(1.hour())?,
                Self::Day => local.checked_add(1.day())?,
                Self::Week => local.checked_add(1.week())?,
            };
        }
    }

    fn floor(self, local: DateTime) -> DateTime {
        match self {
            Self::Hour => local.date().at(local.hour(), 0, 0, 0),
            Self::Day => local.date().to_datetime(jiff::civil::Time::midnight()),
            Self::Week => monday_of(local.date()).to_datetime(jiff::civil::Time::midnight()),
        }
    }

    fn key(self, local: DateTime) -> String {
        match self {
            Self::Hour => local.strftime("%Y-%m-%dT%H").to_string(),
            Self::Day | Self::Week => local.date().to_string(),
        }
    }
}

fn monday_of(date: Date) -> Date {
    date.saturating_sub(i64::from(date.weekday().to_monday_zero_offset()).days())
}
