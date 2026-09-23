//! How a value is written into a column and read back out.

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use jiff::fmt::temporal::DateTimePrinter;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteTypeInfo, SqliteValueRef};
use sqlx::{Database as SqlxDatabase, Decode, Encode, Sqlite, Type};

/// Nine fractional digits and a `Z`, always. The same instant printed to fewer digits
/// sorts differently as text, and every range filter, every ordering and every bucket key
/// in this database reads the column as text.
const PRINTER: DateTimePrinter = DateTimePrinter::new().precision(Some(9));

/// An instant as the database keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredTimestamp(pub Timestamp);

impl From<Timestamp> for StoredTimestamp {
    fn from(value: Timestamp) -> Self {
        Self(value)
    }
}

impl From<StoredTimestamp> for Timestamp {
    fn from(value: StoredTimestamp) -> Self {
        value.0
    }
}

impl fmt::Display for StoredTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&PRINTER.timestamp_to_string(&self.0))
    }
}

impl FromStr for StoredTimestamp {
    type Err = jiff::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

impl Type<Sqlite> for StoredTimestamp {
    fn type_info() -> SqliteTypeInfo {
        <str as Type<Sqlite>>::type_info()
    }

    fn compatible(info: &SqliteTypeInfo) -> bool {
        <str as Type<Sqlite>>::compatible(info)
    }
}

impl<'r> Decode<'r, Sqlite> for StoredTimestamp {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(<&str as Decode<'r, Sqlite>>::decode(value)?.parse()?)
    }
}

impl<'q> Encode<'q, Sqlite> for StoredTimestamp {
    fn encode_by_ref(
        &self,
        buffer: &mut <Sqlite as SqlxDatabase>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <String as Encode<'q, Sqlite>>::encode(self.to_string(), buffer)
    }
}
