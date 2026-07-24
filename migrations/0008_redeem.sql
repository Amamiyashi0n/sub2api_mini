CREATE TABLE IF NOT EXISTS redeem_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    code_prefix TEXT NOT NULL,
    code_hash TEXT NOT NULL UNIQUE,
    plan_id INTEGER NOT NULL REFERENCES plans(id) ON DELETE RESTRICT,
    token_limit INTEGER,
    duration_days INTEGER,
    max_uses INTEGER NOT NULL DEFAULT 1 CHECK (max_uses > 0),
    used_count INTEGER NOT NULL DEFAULT 0 CHECK (used_count >= 0),
    expires_at TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_redeem_codes_hash ON redeem_codes(code_hash);
CREATE INDEX IF NOT EXISTS idx_redeem_codes_available
    ON redeem_codes(enabled, expires_at, used_count, max_uses);

CREATE TABLE IF NOT EXISTS redemptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    redeem_code_id INTEGER NOT NULL REFERENCES redeem_codes(id) ON DELETE RESTRICT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    subscription_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE RESTRICT,
    redeemed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (redeem_code_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_redemptions_user ON redemptions(user_id, redeemed_at DESC);
