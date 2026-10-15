//! Stretches of the viewer's local time that a series is cut into.

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use sqlx::{QueryBuilder, Sqlite};

use crate::analytics::filter::Shift;
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
    pub fn push_expression(self, builder: &mut QueryBuilder<Sqlite>, shift: &Shift) {
        let (head, tail) = match self {
            Self::Hour => ("strftime('%Y-%m-%dT%H', ", ")"),
            Self::Day => ("date(", ")"),
            Self::Week => ("date(", ", 'weekday 0', '-6 days')"),
        };
        builder
            .push(head)
            .push("substr(usage_event.occurred_at, 1, 19), ");
        shift.push(builder);
        builder.push(tail);
    }

    /// Every bucket that overlaps `[from, to)`, in order, keyed as
    /// [`Self::push_expression`] keys them. A bucket the clocks skip has none, and an hour
    /// they repeat is one bucket from the first time it starts.
    pub fn starts(
        self,
        from: Timestamp,
        to: Timestamp,
        time_zone: &TimeZone,
    ) -> Result<Vec<BucketStart>, AnalyticsError> {
        let mut local = self.floor(time_zone.to_datetime(from));
        let mut starts = Vec::new();
        loop {
            if let Some(start) = self.first_instant(local, time_zone)? {
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
            }
            local = match self {
                Self::Hour => local.checked_add(1.hour())?,
                Self::Day => local.checked_add(1.day())?,
                Self::Week => local.checked_add(1.week())?,
            };
        }
    }

    /// The first instant whose local time falls in the bucket that starts at `local`. A
    /// start inside a gap maps past the transition, and the transition itself may already
    /// be in the bucket: a gap from 02:45 to 03:45 leaves the hour 03 a quarter of an hour.
    fn first_instant(
        self,
        local: DateTime,
        time_zone: &TimeZone,
    ) -> Result<Option<Timestamp>, AnalyticsError> {
        let inside = |instant: Timestamp| self.floor(time_zone.to_datetime(instant)) == local;
        let mapped = time_zone.to_timestamp(local)?;
        if inside(mapped) {
            return Ok(Some(mapped));
        }

        Ok(time_zone
            .preceding(mapped)
            .next()
            .map(|transition| transition.timestamp())
            .filter(|instant| inside(*instant)))
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
