ALTER TABLE channel_model_pricing ADD COLUMN cache_read_microusd_per_million INTEGER
    CHECK (cache_read_microusd_per_million IS NULL OR cache_read_microusd_per_million >= 0);
ALTER TABLE channel_model_pricing ADD COLUMN cache_write_microusd_per_million INTEGER
    CHECK (cache_write_microusd_per_million IS NULL OR cache_write_microusd_per_million >= 0);

CREATE TABLE IF NOT EXISTS channel_pricing_intervals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pricing_id INTEGER NOT NULL REFERENCES channel_model_pricing(id) ON DELETE CASCADE,
    min_tokens INTEGER NOT NULL CHECK (min_tokens >= 0),
    max_tokens INTEGER CHECK (max_tokens IS NULL OR max_tokens > min_tokens),
    input_microusd_per_million INTEGER CHECK (input_microusd_per_million IS NULL OR input_microusd_per_million >= 0),
    output_microusd_per_million INTEGER CHECK (output_microusd_per_million IS NULL OR output_microusd_per_million >= 0),
    cache_read_microusd_per_million INTEGER CHECK (cache_read_microusd_per_million IS NULL OR cache_read_microusd_per_million >= 0),
    cache_write_microusd_per_million INTEGER CHECK (cache_write_microusd_per_million IS NULL OR cache_write_microusd_per_million >= 0),
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_channel_pricing_intervals_lookup
    ON channel_pricing_intervals(pricing_id, min_tokens, max_tokens);
