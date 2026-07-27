ALTER TABLE accounts ADD COLUMN platform TEXT NOT NULL DEFAULT 'openai'
    CHECK (platform IN ('openai', 'anthropic'));

ALTER TABLE accounts ADD COLUMN account_type TEXT NOT NULL DEFAULT 'api_key'
    CHECK (account_type IN ('api_key', 'oauth', 'setup_token'));

UPDATE accounts
SET account_type = CASE WHEN kind = 'oauth' THEN 'oauth' ELSE 'api_key' END;

CREATE INDEX IF NOT EXISTS idx_accounts_platform_schedule
    ON accounts(platform, enabled, priority, cooldown_until);
