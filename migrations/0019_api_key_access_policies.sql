ALTER TABLE api_keys ADD COLUMN ip_whitelist TEXT NOT NULL DEFAULT '[]';
ALTER TABLE api_keys ADD COLUMN ip_blacklist TEXT NOT NULL DEFAULT '[]';
ALTER TABLE api_keys ADD COLUMN last_used_ip TEXT;
ALTER TABLE api_keys ADD COLUMN quota_cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK (quota_cost_microusd >= 0);
ALTER TABLE api_keys ADD COLUMN quota_reset_at TEXT;
ALTER TABLE api_keys ADD COLUMN rate_limit_5h_microusd INTEGER NOT NULL DEFAULT 0 CHECK (rate_limit_5h_microusd >= 0);
ALTER TABLE api_keys ADD COLUMN rate_limit_1d_microusd INTEGER NOT NULL DEFAULT 0 CHECK (rate_limit_1d_microusd >= 0);
ALTER TABLE api_keys ADD COLUMN rate_limit_7d_microusd INTEGER NOT NULL DEFAULT 0 CHECK (rate_limit_7d_microusd >= 0);
ALTER TABLE api_keys ADD COLUMN rate_usage_reset_at TEXT;
ALTER TABLE api_keys ADD COLUMN updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP;

CREATE INDEX IF NOT EXISTS idx_usage_key_created_cost
    ON usage_logs(api_key_id, created_at DESC, cost_microusd);
