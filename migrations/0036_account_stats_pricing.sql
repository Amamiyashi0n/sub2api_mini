ALTER TABLE channels ADD COLUMN apply_pricing_to_account_stats INTEGER NOT NULL DEFAULT 0
    CHECK (apply_pricing_to_account_stats IN (0, 1));

ALTER TABLE usage_logs ADD COLUMN account_cost_microusd INTEGER NOT NULL DEFAULT 0
    CHECK (account_cost_microusd >= 0);

UPDATE usage_logs SET account_cost_microusd = cost_microusd;

CREATE TABLE channel_account_stats_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE channel_account_stats_rule_groups (
    rule_id INTEGER NOT NULL REFERENCES channel_account_stats_rules(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    PRIMARY KEY (rule_id, group_id)
);

CREATE TABLE channel_account_stats_rule_accounts (
    rule_id INTEGER NOT NULL REFERENCES channel_account_stats_rules(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    PRIMARY KEY (rule_id, account_id)
);

CREATE TABLE channel_account_stats_pricing (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id INTEGER NOT NULL REFERENCES channel_account_stats_rules(id) ON DELETE CASCADE,
    platform TEXT NOT NULL DEFAULT 'openai',
    models TEXT NOT NULL DEFAULT '[]',
    billing_mode TEXT NOT NULL DEFAULT 'tokens' CHECK (billing_mode IN ('tokens', 'request')),
    input_microusd_per_million INTEGER NOT NULL DEFAULT 0 CHECK (input_microusd_per_million >= 0),
    output_microusd_per_million INTEGER NOT NULL DEFAULT 0 CHECK (output_microusd_per_million >= 0),
    per_request_microusd INTEGER NOT NULL DEFAULT 0 CHECK (per_request_microusd >= 0),
    cache_read_microusd_per_million INTEGER CHECK (cache_read_microusd_per_million IS NULL OR cache_read_microusd_per_million >= 0),
    cache_write_microusd_per_million INTEGER CHECK (cache_write_microusd_per_million IS NULL OR cache_write_microusd_per_million >= 0),
    image_input_microusd_per_million INTEGER CHECK (image_input_microusd_per_million IS NULL OR image_input_microusd_per_million >= 0),
    image_output_microusd_per_million INTEGER CHECK (image_output_microusd_per_million IS NULL OR image_output_microusd_per_million >= 0)
);

CREATE TABLE channel_account_stats_intervals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pricing_id INTEGER NOT NULL REFERENCES channel_account_stats_pricing(id) ON DELETE CASCADE,
    min_tokens INTEGER NOT NULL DEFAULT 0 CHECK (min_tokens >= 0),
    max_tokens INTEGER CHECK (max_tokens IS NULL OR max_tokens > min_tokens),
    input_microusd_per_million INTEGER CHECK (input_microusd_per_million IS NULL OR input_microusd_per_million >= 0),
    output_microusd_per_million INTEGER CHECK (output_microusd_per_million IS NULL OR output_microusd_per_million >= 0),
    cache_read_microusd_per_million INTEGER CHECK (cache_read_microusd_per_million IS NULL OR cache_read_microusd_per_million >= 0),
    cache_write_microusd_per_million INTEGER CHECK (cache_write_microusd_per_million IS NULL OR cache_write_microusd_per_million >= 0),
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_account_stats_rules_channel
    ON channel_account_stats_rules(channel_id, sort_order, id);
CREATE INDEX idx_account_stats_rule_groups_group
    ON channel_account_stats_rule_groups(group_id, rule_id);
CREATE INDEX idx_account_stats_rule_accounts_account
    ON channel_account_stats_rule_accounts(account_id, rule_id);
CREATE INDEX idx_account_stats_pricing_rule
    ON channel_account_stats_pricing(rule_id, id);
CREATE INDEX idx_account_stats_intervals_pricing
    ON channel_account_stats_intervals(pricing_id, min_tokens, sort_order);
CREATE INDEX idx_usage_account_cost
    ON usage_logs(account_id, created_at DESC, account_cost_microusd);
