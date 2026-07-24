ALTER TABLE usage_logs ADD COLUMN ttft_ms INTEGER CHECK (ttft_ms IS NULL OR ttft_ms >= 0);
ALTER TABLE usage_logs ADD COLUMN upstream_attempts INTEGER NOT NULL DEFAULT 1
    CHECK (upstream_attempts >= 0);
ALTER TABLE usage_logs ADD COLUMN account_switches INTEGER NOT NULL DEFAULT 0
    CHECK (account_switches >= 0);

CREATE TABLE ops_minute_rollups (
    bucket TEXT PRIMARY KEY,
    requests INTEGER NOT NULL DEFAULT 0,
    successes INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    tokens INTEGER NOT NULL DEFAULT 0,
    cost_microusd INTEGER NOT NULL DEFAULT 0,
    duration_sum_ms INTEGER NOT NULL DEFAULT 0,
    duration_max_ms INTEGER NOT NULL DEFAULT 0,
    ttft_sum_ms INTEGER NOT NULL DEFAULT 0,
    ttft_count INTEGER NOT NULL DEFAULT 0,
    ttft_max_ms INTEGER NOT NULL DEFAULT 0,
    account_switches INTEGER NOT NULL DEFAULT 0,
    stream_requests INTEGER NOT NULL DEFAULT 0,
    stream_duration_sum_ms INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE runtime_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    level TEXT NOT NULL CHECK (level IN ('trace', 'debug', 'info', 'warn', 'error')),
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    request_id TEXT,
    fields_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_runtime_logs_created ON runtime_logs(created_at DESC, id DESC);
CREATE INDEX idx_runtime_logs_level ON runtime_logs(level, created_at DESC);

CREATE TABLE ops_report_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    report_type TEXT NOT NULL CHECK (report_type IN ('daily', 'weekly', 'manual')),
    period_start TEXT NOT NULL,
    period_end TEXT NOT NULL,
    recipients TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL CHECK (status IN ('processing', 'sent', 'skipped', 'failed')),
    metrics_json TEXT NOT NULL DEFAULT '{}',
    error_summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT,
    UNIQUE(report_type, period_start, period_end)
);
CREATE INDEX idx_ops_report_runs_created ON ops_report_runs(created_at DESC, id DESC);

INSERT OR IGNORE INTO app_settings (key, value) VALUES
    ('runtime_log_level', 'info'),
    ('runtime_log_db_enabled', 'true');
