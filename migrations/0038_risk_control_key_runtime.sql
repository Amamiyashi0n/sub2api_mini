CREATE TABLE IF NOT EXISTS risk_control_api_key_runtime (
    key_hash TEXT PRIMARY KEY,
    masked TEXT NOT NULL DEFAULT '',
    failure_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT '',
    last_checked_at TEXT,
    frozen_until TEXT,
    last_latency_ms INTEGER NOT NULL DEFAULT 0,
    last_http_status INTEGER NOT NULL DEFAULT 0,
    last_tested INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    successes INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    latency_total_ms INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_risk_control_api_key_runtime_frozen
    ON risk_control_api_key_runtime(frozen_until);
