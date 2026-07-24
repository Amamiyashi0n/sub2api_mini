CREATE TABLE IF NOT EXISTS channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'inactive')),
    restrict_models INTEGER NOT NULL DEFAULT 0 CHECK (restrict_models IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS channel_groups (
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (channel_id, group_id),
    UNIQUE (group_id)
);

CREATE TABLE IF NOT EXISTS channel_model_pricing (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    platform TEXT NOT NULL DEFAULT 'openai',
    models TEXT NOT NULL DEFAULT '[]',
    billing_mode TEXT NOT NULL DEFAULT 'tokens' CHECK (billing_mode IN ('tokens', 'request')),
    input_microusd_per_million INTEGER NOT NULL DEFAULT 0 CHECK (input_microusd_per_million >= 0),
    output_microusd_per_million INTEGER NOT NULL DEFAULT 0 CHECK (output_microusd_per_million >= 0),
    per_request_microusd INTEGER NOT NULL DEFAULT 0 CHECK (per_request_microusd >= 0)
);

CREATE INDEX IF NOT EXISTS idx_channel_pricing_channel ON channel_model_pricing(channel_id);
ALTER TABLE usage_logs ADD COLUMN cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK (cost_microusd >= 0);
CREATE INDEX IF NOT EXISTS idx_usage_cost ON usage_logs(created_at, cost_microusd);
