ALTER TABLE announcements ADD COLUMN targeting TEXT NOT NULL DEFAULT '{"any_of":[]}'
    CHECK (json_valid(targeting));

ALTER TABLE promo_codes ADD COLUMN notes TEXT NOT NULL DEFAULT '';
ALTER TABLE promo_codes ADD COLUMN unlimited_uses INTEGER NOT NULL DEFAULT 0
    CHECK (unlimited_uses IN (0, 1));
ALTER TABLE promo_codes ADD COLUMN updated_at TEXT;

UPDATE promo_codes SET updated_at = created_at WHERE updated_at IS NULL;

CREATE INDEX idx_promo_codes_status_updated
    ON promo_codes(enabled, updated_at DESC, id DESC);
