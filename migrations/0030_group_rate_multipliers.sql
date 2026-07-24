ALTER TABLE groups ADD COLUMN platform TEXT NOT NULL DEFAULT 'openai';
ALTER TABLE groups ADD COLUMN is_exclusive INTEGER NOT NULL DEFAULT 0 CHECK (is_exclusive IN (0, 1));
ALTER TABLE groups ADD COLUMN subscription_type TEXT NOT NULL DEFAULT 'standard'
    CHECK (subscription_type IN ('standard', 'subscription'));
ALTER TABLE groups ADD COLUMN rate_multiplier_micros INTEGER NOT NULL DEFAULT 1000000
    CHECK (rate_multiplier_micros > 0 AND rate_multiplier_micros <= 1000000000);
ALTER TABLE groups ADD COLUMN peak_rate_enabled INTEGER NOT NULL DEFAULT 0
    CHECK (peak_rate_enabled IN (0, 1));
ALTER TABLE groups ADD COLUMN peak_start TEXT NOT NULL DEFAULT '';
ALTER TABLE groups ADD COLUMN peak_end TEXT NOT NULL DEFAULT '';
ALTER TABLE groups ADD COLUMN peak_rate_multiplier_micros INTEGER NOT NULL DEFAULT 1000000
    CHECK (peak_rate_multiplier_micros >= 0 AND peak_rate_multiplier_micros <= 1000000000);

CREATE TABLE IF NOT EXISTS user_group_rate_multipliers (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    rate_multiplier_micros INTEGER NOT NULL
        CHECK (rate_multiplier_micros > 0 AND rate_multiplier_micros <= 1000000000),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, group_id)
);

CREATE INDEX IF NOT EXISTS idx_user_group_rates_group
    ON user_group_rate_multipliers(group_id, user_id);
