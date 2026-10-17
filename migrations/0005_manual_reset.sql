CREATE TABLE manual_reset (
    account_id INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL
);

CREATE INDEX manual_reset_account ON manual_reset (account_id, expires_at);
