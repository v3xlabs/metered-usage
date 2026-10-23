-- Human requests are retained separately from value-change events. An unfinished action
-- has an unknown outcome after a crash and must never cause an automatic retry.
CREATE TABLE quota_action (
    action_id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    account_id TEXT,
    source_id TEXT,
    requested_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT,
    before_state TEXT NOT NULL,
    after_state TEXT,
    requested_by TEXT NOT NULL DEFAULT 'unspecified',
    shared INTEGER NOT NULL DEFAULT 0,
    response_status INTEGER,
    response_body TEXT,
    -- Credential roles are labels only, never tokens or password verifiers.
    credential_id TEXT NOT NULL DEFAULT 'legacy',
    credential_transport TEXT NOT NULL DEFAULT 'unknown',
    -- A live worker owns its action until completion, regardless of elapsed time.
    boot_id TEXT
);
CREATE INDEX quota_action_scope ON quota_action(kind, account_id, source_id, action_id);

CREATE TABLE quota_action_request (
    request_id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id INTEGER NOT NULL REFERENCES quota_action(action_id),
    requested_by TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    credential_id TEXT NOT NULL DEFAULT 'legacy',
    credential_transport TEXT NOT NULL DEFAULT 'unknown'
);
CREATE INDEX quota_action_request_action ON quota_action_request(action_id, request_id);

CREATE TABLE quota_action_key (
    credential_id TEXT NOT NULL,
    key TEXT NOT NULL,
    kind TEXT NOT NULL,
    account_id TEXT,
    source_id TEXT,
    action_id INTEGER NOT NULL REFERENCES quota_action(action_id),
    PRIMARY KEY (credential_id, key)
);
