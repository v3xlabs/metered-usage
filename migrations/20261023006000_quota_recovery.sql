-- Reserving the next attempt before dispatch preserves backoff across crashes/restarts.
CREATE TABLE quota_recovery (
    account_id INTEGER PRIMARY KEY REFERENCES account(account_id) ON DELETE CASCADE,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 8),
    next_attempt_at TEXT NOT NULL
);
