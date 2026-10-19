-- This table now stores grants. One Claude grant can hold several resets under one label and
-- may have no expiry date. A Codex credit is a grant of one reset, with no label or total.
CREATE TABLE manual_reset_grant (
    account_id INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    label TEXT,
    remaining INTEGER NOT NULL,
    total INTEGER,
    expires_at TEXT
);

INSERT INTO manual_reset_grant (account_id, remaining, expires_at)
SELECT account_id, 1, expires_at FROM manual_reset;

DROP TABLE manual_reset;
ALTER TABLE manual_reset_grant RENAME TO manual_reset;
CREATE INDEX manual_reset_account ON manual_reset (account_id, expires_at);
