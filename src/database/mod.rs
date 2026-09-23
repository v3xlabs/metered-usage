//! The connection to SQLite and the errors reading it can produce. Queries do not live
//! here: each one is a method on the type it reads or writes.

pub mod codec;

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::id::IdGenerator;

pub struct Database {
    pub pool: SqlitePool,
    pub ids: IdGenerator,
}

impl Database {
    pub async fn open(url: &str, node: u16) -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            // A write-ahead log already survives a process that dies, and a lost record is
            // one line of a gateway log that can be posted again, so waiting for the
            // platter on each commit buys nothing and costs seconds on a network volume.
            .synchronous(SqliteSynchronous::Normal)
            // SQLite asks the filesystem for a temp directory when a sorter or a temp
            // b-tree outgrows its cache, and answers `SQLITE_IOERR_GETTEMPPATH` when no
            // candidate in `SQLITE_TMPDIR`, `TMPDIR`, `/var/tmp`, `/usr/tmp`, `/tmp`, `.`
            // is writable. The container has a read-only root filesystem and `/` for a
            // working directory, so none of them are, and only the writable volume holding
            // this file is. Sorting in memory needs no such directory.
            .pragma("temp_store", "MEMORY")
            .busy_timeout(Duration::from_secs(10));
        let max_connections = if url == "sqlite::memory:" { 1 } else { 4 };
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self {
            pool,
            ids: IdGenerator::new(node),
        })
    }

    /// A transaction that will write before it commits.
    ///
    /// Every one of ours reads and then writes. Started deferred, the write lock is taken
    /// late, and SQLite answers a late upgrade with `SQLITE_BUSY` immediately instead of
    /// waiting out `busy_timeout`, so a reader that arrives while the queue is writing is
    /// refused rather than delayed. Claiming the writer up front makes the wait happen.
    pub async fn write(&self) -> Result<Transaction<'static, Sqlite>, DatabaseError> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("database: {0}")]
    Query(#[from] sqlx::Error),
    #[error("migration: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("stored {field} is not readable: {value:?}")]
    Unreadable { field: &'static str, value: String },
}
