-- A gateway declared in the config file. `key` is the config's own id for it, so a restart
-- with the same config finds the same row. A source dropped from the config is disabled,
-- never deleted, because its events are history.
CREATE TABLE source (
    source_id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    base_url TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

-- What the resident collector for a source last managed. One row per source it has run for.
CREATE TABLE collector_state (
    source_id INTEGER PRIMARY KEY REFERENCES source (source_id) ON DELETE CASCADE,
    connected_since TEXT,
    last_record_at TEXT,
    records_received INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    updated_at TEXT NOT NULL
);

-- The upstream credential whose quota a request consumed. It is not the email: two
-- credential files that log in as one person are two accounts. `upstream_key` is the
-- gateway's own name for the credential, CLIProxy's `auth_index`, which is unique per
-- credential within a source. `provider` is an attribute, not part of the key, because a
-- usage record and the gateway's credential list need not name it alike. A record that
-- names no credential gets `unattributed/<provider>`, one bucket per provider.
-- A merged account keeps its row so a later record for its key still resolves, through
-- `merged_into_account_id`, to the account that absorbed it.
CREATE TABLE account (
    account_id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES source (source_id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    upstream_key TEXT NOT NULL,
    auth_kind TEXT NOT NULL,
    label TEXT,
    display_name TEXT,
    merged_into_account_id INTEGER REFERENCES account (account_id),
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    UNIQUE (source_id, upstream_key)
);

-- The last state the gateway reported for a credential. Soft columns come from reading the
-- gateway's own credential list; hard columns from asking the provider through it.
CREATE TABLE credential_state (
    account_id INTEGER PRIMARY KEY REFERENCES account (account_id) ON DELETE CASCADE,
    status TEXT,
    status_message TEXT,
    disabled INTEGER NOT NULL DEFAULT 0,
    unavailable INTEGER NOT NULL DEFAULT 0,
    next_retry_after TEXT,
    cooldowns TEXT NOT NULL DEFAULT '[]',
    account_type TEXT,
    plan TEXT,
    hard_refreshable INTEGER NOT NULL DEFAULT 0,
    soft_observed_at TEXT,
    hard_refreshed_at TEXT,
    hard_refresh_error TEXT
);

-- One limit window per credential as the provider last described it, replaced on every
-- hard refresh. Units differ by provider, so every row is normalized to a used fraction.
CREATE TABLE quota_window (
    account_id INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    window_key TEXT NOT NULL,
    label TEXT NOT NULL,
    used_fraction REAL,
    used_value REAL,
    limit_value REAL,
    unit TEXT,
    window_seconds INTEGER,
    resets_at TEXT,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (account_id, window_key)
);

-- A price is never edited in place. A change is a new row with a later `effective_from`,
-- so an event priced last month keeps the row that priced it. `provider` and
-- `service_tier` NULL mean any; `min_input_tokens` is the long-context threshold the row
-- starts at. Among matching rows the origin ranks manual over litellm_live over
-- litellm_public, then the latest `effective_from` not after the event wins.
CREATE TABLE model_price (
    price_id INTEGER PRIMARY KEY,
    provider TEXT,
    model TEXT NOT NULL,
    service_tier TEXT,
    min_input_tokens INTEGER NOT NULL DEFAULT 0,
    origin TEXT NOT NULL,
    effective_from TEXT NOT NULL,
    input_usd_per_mtok REAL NOT NULL,
    cached_input_usd_per_mtok REAL NOT NULL,
    cache_write_usd_per_mtok REAL NOT NULL,
    output_usd_per_mtok REAL NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX model_price_lookup ON model_price (model, effective_from);

-- A subscription paid for an account. Its price is never spread over requests; it is only
-- set against the list cost of each billing period for leverage.
CREATE TABLE plan (
    plan_id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    monthly_usd REAL NOT NULL,
    period_start TEXT NOT NULL,
    period_end TEXT
);

-- `input_tokens` counts every input token, cached or not; `cache_read_tokens` and
-- `cache_write_tokens` are the parts of it that hit or filled a cache. `token_quality` is the
-- gateway's own verdict on that partition, and `unclassified_tokens` holds whatever it could
-- not place, which a breakdown it calls inconsistent puts all of its total into.
CREATE TABLE usage_event (
    event_id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES source (source_id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    record_hash TEXT NOT NULL,
    upstream_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    model_alias TEXT,
    endpoint TEXT NOT NULL,
    caller TEXT,
    harness TEXT,
    user_agent TEXT,
    session_id TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    unclassified_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    token_quality TEXT,
    latency_ms INTEGER,
    ttft_ms INTEGER,
    streamed INTEGER NOT NULL,
    status_code INTEGER NOT NULL,
    failed INTEGER NOT NULL,
    error_message TEXT,
    service_tier TEXT,
    reasoning_effort TEXT,
    price_id INTEGER REFERENCES model_price (price_id),
    list_cost_usd REAL,
    billed_cost_usd REAL
);

-- The record hash is taken over the normalized record, so the same record posted or popped
-- twice is one row while two requests that only share an upstream id are two.
CREATE UNIQUE INDEX usage_event_record ON usage_event (source_id, record_hash);

CREATE INDEX usage_event_occurred_at ON usage_event (occurred_at);

CREATE INDEX usage_event_account ON usage_event (account_id, occurred_at);

CREATE INDEX usage_event_unpriced ON usage_event (event_id) WHERE list_cost_usd IS NULL;

-- A record the normalizer refused. The queue read upstream is destructive, so this is the
-- only copy left. Secrets are stripped from `payload` before it is written.
CREATE TABLE dead_letter (
    dead_letter_id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES source (source_id) ON DELETE CASCADE,
    received_at TEXT NOT NULL,
    error TEXT NOT NULL,
    payload TEXT NOT NULL
);

CREATE INDEX dead_letter_received_at ON dead_letter (received_at);
