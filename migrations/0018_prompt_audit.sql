INSERT INTO app_settings (key, value, updated_at)
VALUES ('prompt_audit_config', '{}', CURRENT_TIMESTAMP)
ON CONFLICT(key) DO NOTHING;

CREATE TABLE IF NOT EXISTS prompt_audit_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL DEFAULT '',
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    username_snapshot TEXT NOT NULL DEFAULT '',
    user_email_snapshot TEXT NOT NULL DEFAULT '',
    api_key_id INTEGER REFERENCES api_keys(id) ON DELETE SET NULL,
    api_key_name_snapshot TEXT NOT NULL DEFAULT '',
    group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    group_name TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL DEFAULT 'openai',
    endpoint TEXT NOT NULL DEFAULT '',
    protocol TEXT NOT NULL DEFAULT 'openai_compatible',
    model TEXT NOT NULL DEFAULT '',
    prompt_hash TEXT NOT NULL DEFAULT '',
    redacted_preview TEXT NOT NULL DEFAULT '',
    prompt_length INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    stage TEXT NOT NULL DEFAULT 'http',
    execution_mode TEXT NOT NULL DEFAULT 'async_audit'
        CHECK (execution_mode IN ('async_audit', 'blocking')),
    config_version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('staging', 'queued', 'processing', 'retry', 'done', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error_code TEXT NOT NULL DEFAULT '',
    last_error_message TEXT NOT NULL DEFAULT '',
    processing_started_at TEXT,
    processed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_prompt_audit_jobs_status
    ON prompt_audit_jobs(status, id);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_jobs_request
    ON prompt_audit_jobs(request_id);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_jobs_created
    ON prompt_audit_jobs(created_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS prompt_audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES prompt_audit_jobs(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL DEFAULT '',
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    username_snapshot TEXT NOT NULL DEFAULT '',
    user_email_snapshot TEXT NOT NULL DEFAULT '',
    api_key_id INTEGER REFERENCES api_keys(id) ON DELETE SET NULL,
    api_key_name_snapshot TEXT NOT NULL DEFAULT '',
    group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    group_name TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL DEFAULT 'openai',
    endpoint TEXT NOT NULL DEFAULT '',
    protocol TEXT NOT NULL DEFAULT 'openai_compatible',
    model TEXT NOT NULL DEFAULT '',
    prompt_hash TEXT NOT NULL DEFAULT '',
    redacted_preview TEXT NOT NULL DEFAULT '',
    full_prompt TEXT NOT NULL DEFAULT '',
    prompt_length INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    stage TEXT NOT NULL DEFAULT 'http',
    decision TEXT NOT NULL DEFAULT 'pass'
        CHECK (decision IN ('pass', 'flag', 'critical')),
    risk_level TEXT NOT NULL DEFAULT 'low'
        CHECK (risk_level IN ('low', 'medium', 'high', 'critical')),
    action TEXT NOT NULL DEFAULT 'Allow'
        CHECK (action IN ('Allow', 'Warn', 'Block')),
    categories TEXT NOT NULL DEFAULT '[]',
    matched_scanners TEXT NOT NULL DEFAULT '[]',
    scanner_scores TEXT NOT NULL DEFAULT '{}',
    scanner_evidence TEXT NOT NULL DEFAULT '{}',
    scanner_backend TEXT NOT NULL DEFAULT 'qwen3guard-openai',
    scanner_version TEXT NOT NULL DEFAULT 'qwen3guard',
    guard_endpoint_id TEXT NOT NULL DEFAULT '',
    policy_id TEXT NOT NULL DEFAULT 'priority',
    policy_version INTEGER NOT NULL DEFAULT 1,
    config_version INTEGER NOT NULL DEFAULT 1,
    chunk_total INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    issue_summaries TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_job
    ON prompt_audit_events(job_id);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_request
    ON prompt_audit_events(request_id);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_decision
    ON prompt_audit_events(decision, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_risk
    ON prompt_audit_events(risk_level, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_user
    ON prompt_audit_events(user_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_key
    ON prompt_audit_events(api_key_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_group
    ON prompt_audit_events(group_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_hash
    ON prompt_audit_events(prompt_hash);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_events_created
    ON prompt_audit_events(created_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS prompt_audit_probe_results (
    endpoint_id TEXT PRIMARY KEY,
    ok INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT '',
    error_code TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL DEFAULT '',
    latency_ms INTEGER NOT NULL DEFAULT 0,
    http_status INTEGER NOT NULL DEFAULT 0,
    retryable INTEGER NOT NULL DEFAULT 0,
    checked_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    token_applied INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS prompt_audit_delete_previews (
    token_hash TEXT PRIMARY KEY,
    admin_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    filter_hash TEXT NOT NULL,
    filter_json TEXT NOT NULL,
    snapshot_max_id INTEGER NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_prompt_audit_delete_previews_expiry
    ON prompt_audit_delete_previews(expires_at);
