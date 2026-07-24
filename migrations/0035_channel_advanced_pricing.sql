ALTER TABLE channels ADD COLUMN model_mapping TEXT NOT NULL DEFAULT '{}';
ALTER TABLE channels ADD COLUMN billing_model_source TEXT NOT NULL DEFAULT 'channel_mapped'
    CHECK (billing_model_source IN ('requested', 'upstream', 'channel_mapped'));

ALTER TABLE channel_model_pricing ADD COLUMN image_input_microusd_per_million INTEGER
    CHECK (image_input_microusd_per_million IS NULL OR image_input_microusd_per_million >= 0);
ALTER TABLE channel_model_pricing ADD COLUMN image_output_microusd_per_million INTEGER
    CHECK (image_output_microusd_per_million IS NULL OR image_output_microusd_per_million >= 0);

ALTER TABLE usage_logs ADD COLUMN cache_write_tokens INTEGER NOT NULL DEFAULT 0
    CHECK (cache_write_tokens >= 0);
ALTER TABLE usage_logs ADD COLUMN image_input_tokens INTEGER NOT NULL DEFAULT 0
    CHECK (image_input_tokens >= 0);
ALTER TABLE usage_logs ADD COLUMN image_output_tokens INTEGER NOT NULL DEFAULT 0
    CHECK (image_output_tokens >= 0);
ALTER TABLE usage_logs ADD COLUMN billing_model TEXT;
ALTER TABLE usage_logs ADD COLUMN mapped_model TEXT;
ALTER TABLE usage_logs ADD COLUMN model_mapping_chain TEXT NOT NULL DEFAULT '';

CREATE INDEX idx_usage_billing_model
    ON usage_logs(billing_model, created_at DESC);
