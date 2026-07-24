CREATE TABLE IF NOT EXISTS channel_monitors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('openai', 'anthropic', 'gemini', 'grok')),
    api_mode TEXT NOT NULL DEFAULT 'chat_completions' CHECK (api_mode IN ('chat_completions', 'responses')),
    endpoint TEXT NOT NULL,
    encrypted_request_config TEXT NOT NULL,
    primary_model TEXT NOT NULL,
    extra_models TEXT NOT NULL DEFAULT '[]',
    group_name TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    interval_seconds INTEGER NOT NULL DEFAULT 300 CHECK (interval_seconds BETWEEN 30 AND 86400),
    jitter_seconds INTEGER NOT NULL DEFAULT 0 CHECK (jitter_seconds BETWEEN 0 AND 3600),
    last_checked_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_channel_monitors_due
    ON channel_monitors(enabled, last_checked_at);

CREATE TABLE IF NOT EXISTS channel_monitor_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    monitor_id INTEGER NOT NULL REFERENCES channel_monitors(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('operational', 'degraded', 'failed', 'error')),
    latency_ms INTEGER,
    ping_latency_ms INTEGER,
    message TEXT NOT NULL DEFAULT '',
    checked_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_channel_monitor_history_lookup
    ON channel_monitor_history(monitor_id, model, checked_at DESC);

INSERT OR IGNORE INTO app_settings (key, value) VALUES ('channel_monitor_enabled', 'true');
INSERT OR IGNORE INTO app_settings (key, value) VALUES ('channel_monitor_default_interval_seconds', '300');
