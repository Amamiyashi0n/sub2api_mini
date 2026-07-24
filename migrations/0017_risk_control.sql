INSERT INTO app_settings (key, value, updated_at)
VALUES ('risk_control_config', '{}', CURRENT_TIMESTAMP)
ON CONFLICT(key) DO NOTHING;

CREATE TABLE IF NOT EXISTS risk_control_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL,
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    api_key_id INTEGER REFERENCES api_keys(id) ON DELETE SET NULL,
    group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    endpoint TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    mode TEXT NOT NULL DEFAULT '',
    action TEXT NOT NULL DEFAULT '',
    flagged INTEGER NOT NULL DEFAULT 0,
    highest_category TEXT NOT NULL DEFAULT '',
    highest_score REAL NOT NULL DEFAULT 0,
    matched_keyword TEXT NOT NULL DEFAULT '',
    category_scores TEXT NOT NULL DEFAULT '{}',
    threshold_snapshot TEXT NOT NULL DEFAULT '{}',
    input_hash TEXT NOT NULL DEFAULT '',
    upstream_latency_ms INTEGER,
    error_summary TEXT NOT NULL DEFAULT '',
    violation_count INTEGER NOT NULL DEFAULT 0,
    auto_banned INTEGER NOT NULL DEFAULT 0,
    email_sent INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_risk_control_logs_created
    ON risk_control_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_risk_control_logs_flagged_created
    ON risk_control_logs(flagged, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_risk_control_logs_user_created
    ON risk_control_logs(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_risk_control_logs_key_created
    ON risk_control_logs(api_key_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_risk_control_logs_group_created
    ON risk_control_logs(group_id, created_at DESC);

CREATE TABLE IF NOT EXISTS risk_control_hashes (
    input_hash TEXT PRIMARY KEY,
    first_log_id INTEGER REFERENCES risk_control_logs(id) ON DELETE SET NULL,
    hit_count INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
