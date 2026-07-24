ALTER TABLE usage_logs ADD COLUMN cached_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cached_input_tokens >= 0);
ALTER TABLE usage_logs ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_tokens >= 0);
ALTER TABLE usage_logs ADD COLUMN request_type TEXT NOT NULL DEFAULT 'sync' CHECK (request_type IN ('sync', 'stream'));
ALTER TABLE usage_logs ADD COLUMN stream INTEGER NOT NULL DEFAULT 0 CHECK (stream IN (0, 1));
ALTER TABLE usage_logs ADD COLUMN service_tier TEXT;

CREATE INDEX IF NOT EXISTS idx_usage_request_type
    ON usage_logs(request_type, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_status_created
    ON usage_logs(status_code, created_at DESC);

CREATE TABLE IF NOT EXISTS usage_delete_previews (
    token_hash TEXT PRIMARY KEY,
    admin_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    filter_hash TEXT NOT NULL,
    filter_json TEXT NOT NULL,
    snapshot_max_id INTEGER NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_usage_delete_previews_expiry
    ON usage_delete_previews(expires_at);
