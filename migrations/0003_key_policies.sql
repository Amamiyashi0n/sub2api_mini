ALTER TABLE api_keys ADD COLUMN expires_at TEXT;
ALTER TABLE api_keys ADD COLUMN quota_tokens INTEGER NOT NULL DEFAULT 0 CHECK (quota_tokens >= 0);
ALTER TABLE api_keys ADD COLUMN allowed_models TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS idx_api_keys_expiry ON api_keys(enabled, expires_at);
