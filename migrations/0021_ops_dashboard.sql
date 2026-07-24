CREATE TABLE IF NOT EXISTS ops_alert_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    metric_type TEXT NOT NULL,
    operator TEXT NOT NULL,
    threshold REAL NOT NULL,
    window_minutes INTEGER NOT NULL DEFAULT 5 CHECK (window_minutes BETWEEN 1 AND 1440),
    severity TEXT NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    cooldown_minutes INTEGER NOT NULL DEFAULT 15 CHECK (cooldown_minutes BETWEEN 1 AND 10080),
    notify_email INTEGER NOT NULL DEFAULT 0 CHECK (notify_email IN (0, 1)),
    last_triggered_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS ops_alert_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id INTEGER NOT NULL REFERENCES ops_alert_rules(id) ON DELETE CASCADE,
    severity TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'firing' CHECK (status IN ('firing', 'resolved', 'manual_resolved')),
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    metric_value REAL NOT NULL,
    threshold_value REAL NOT NULL,
    email_sent INTEGER NOT NULL DEFAULT 0 CHECK (email_sent IN (0, 1)),
    fired_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    resolved_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_ops_alert_events_status
    ON ops_alert_events(status, fired_at DESC);
CREATE INDEX IF NOT EXISTS idx_ops_alert_events_rule
    ON ops_alert_events(rule_id, fired_at DESC);

INSERT OR IGNORE INTO app_settings (key, value) VALUES
    ('ops_settings', '{"auto_refresh_seconds":10,"alert_recipients":[],"email_enabled":false,"request_retention_days":90}');
