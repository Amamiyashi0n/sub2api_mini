ALTER TABLE users ADD COLUMN balance_cents INTEGER NOT NULL DEFAULT 0 CHECK (balance_cents >= 0);

CREATE TABLE IF NOT EXISTS promo_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    code_hash TEXT NOT NULL UNIQUE,
    code_prefix TEXT NOT NULL,
    bonus_cents INTEGER NOT NULL CHECK (bonus_cents > 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    max_uses INTEGER NOT NULL DEFAULT 1 CHECK (max_uses > 0),
    used_count INTEGER NOT NULL DEFAULT 0 CHECK (used_count >= 0),
    expires_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS promo_usages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    promo_code_id INTEGER NOT NULL REFERENCES promo_codes(id) ON DELETE RESTRICT,
    user_id INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    bonus_cents INTEGER NOT NULL,
    used_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS affiliate_profiles (
    user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    affiliate_code TEXT NOT NULL UNIQUE COLLATE NOCASE,
    available_cents INTEGER NOT NULL DEFAULT 0 CHECK (available_cents >= 0),
    frozen_cents INTEGER NOT NULL DEFAULT 0 CHECK (frozen_cents >= 0),
    history_cents INTEGER NOT NULL DEFAULT 0 CHECK (history_cents >= 0),
    rebate_rate_bps INTEGER CHECK (rebate_rate_bps BETWEEN 0 AND 10000),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS affiliate_invites (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    inviter_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invitee_id INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (inviter_id != invitee_id)
);

CREATE TABLE IF NOT EXISTS affiliate_rebates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    inviter_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invitee_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id INTEGER,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    status TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('frozen', 'available', 'cancelled')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS affiliate_transfers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_promo_codes_created ON promo_codes(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_promo_usages_code ON promo_usages(promo_code_id, used_at DESC);
CREATE INDEX IF NOT EXISTS idx_affiliate_invites_inviter ON affiliate_invites(inviter_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_affiliate_rebates_inviter ON affiliate_rebates(inviter_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_affiliate_transfers_user ON affiliate_transfers(user_id, created_at DESC);

INSERT OR IGNORE INTO app_settings (key, value) VALUES
    ('promo_code_enabled', 'false'),
    ('affiliate_enabled', 'false'),
    ('affiliate_rebate_rate_bps', '1000');
