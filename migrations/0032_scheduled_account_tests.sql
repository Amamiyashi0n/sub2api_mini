CREATE TABLE scheduled_test_plans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    cron_expression TEXT NOT NULL DEFAULT '*/30 * * * *',
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    max_results INTEGER NOT NULL DEFAULT 50 CHECK (max_results BETWEEN 1 AND 500),
    auto_recover INTEGER NOT NULL DEFAULT 0 CHECK (auto_recover IN (0, 1)),
    last_run_at TEXT,
    next_run_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_scheduled_test_plans_account
    ON scheduled_test_plans(account_id, id DESC);
CREATE INDEX idx_scheduled_test_plans_due
    ON scheduled_test_plans(enabled, next_run_at);

CREATE TABLE scheduled_test_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id INTEGER NOT NULL REFERENCES scheduled_test_plans(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('success', 'failed')),
    response_text TEXT NOT NULL DEFAULT '',
    error_message TEXT NOT NULL DEFAULT '',
    latency_ms INTEGER NOT NULL DEFAULT 0 CHECK (latency_ms >= 0),
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_scheduled_test_results_plan
    ON scheduled_test_results(plan_id, created_at DESC, id DESC);
