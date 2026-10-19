-- What changed in the quota, manual resets, prices and accounts that other services read,
-- in commit order, so they can follow it instead of polling every list. `cursor` is
-- AUTOINCREMENT so that no cursor is handed out twice, even after pruning empties the table.
CREATE TABLE change (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    account_id INTEGER,
    window_key TEXT,
    model TEXT,
    recorded_at TEXT NOT NULL
);

CREATE INDEX change_recorded_at ON change (recorded_at);

-- The highest cursor pruning has deleted. A reader whose cursor is below it may have missed
-- a change and has to read the lists again.
CREATE TABLE change_horizon (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 0),
    pruned_through INTEGER NOT NULL
);

INSERT INTO change_horizon (singleton, pruned_through) VALUES (0, 0);
