-- `scope` names the metered usage that counts toward a window: `account` for all of it,
-- `unmetered` for none of it, `model:<name>` for the models whose name contains `<name>`.
-- A window that `starts_on_use` opens at the first request after it resets, not at the reset.
ALTER TABLE quota_window ADD COLUMN scope TEXT NOT NULL DEFAULT 'account';
ALTER TABLE quota_window ADD COLUMN starts_on_use INTEGER NOT NULL DEFAULT 0;

-- The last report of every window a credential went through, one row per window, kept after
-- the window resets. The list cost metered in each is summed when it is read, so a price
-- corrected later still corrects the share of a window that one list dollar spends.
CREATE TABLE quota_sample (
    account_id INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    window_key TEXT NOT NULL,
    started_at TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    used_fraction REAL NOT NULL,
    PRIMARY KEY (account_id, window_key, started_at)
);
