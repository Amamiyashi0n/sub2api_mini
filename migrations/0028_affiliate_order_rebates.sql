ALTER TABLE affiliate_profiles ADD COLUMN debt_cents INTEGER NOT NULL DEFAULT 0
    CHECK (debt_cents >= 0);

ALTER TABLE affiliate_rebates ADD COLUMN base_amount_cents INTEGER NOT NULL DEFAULT 0
    CHECK (base_amount_cents >= 0);
ALTER TABLE affiliate_rebates ADD COLUMN rate_bps INTEGER NOT NULL DEFAULT 0
    CHECK (rate_bps BETWEEN 0 AND 10000);
ALTER TABLE affiliate_rebates ADD COLUMN available_at TEXT;
ALTER TABLE affiliate_rebates ADD COLUMN cancelled_at TEXT;
ALTER TABLE affiliate_rebates ADD COLUMN cancellation_reason TEXT;

UPDATE affiliate_rebates SET
    base_amount_cents = CASE WHEN base_amount_cents = 0 THEN amount_cents ELSE base_amount_cents END,
    available_at = CASE WHEN status = 'available' THEN created_at ELSE available_at END;

UPDATE affiliate_rebates SET source_id = NULL
WHERE source_type = 'order' AND source_id IS NOT NULL AND id NOT IN (
    SELECT MIN(id) FROM affiliate_rebates
    WHERE source_type = 'order' AND source_id IS NOT NULL GROUP BY source_id
);

CREATE UNIQUE INDEX idx_affiliate_rebates_order_source
    ON affiliate_rebates(source_type, source_id)
    WHERE source_type = 'order' AND source_id IS NOT NULL;
CREATE INDEX idx_affiliate_rebates_release
    ON affiliate_rebates(status, available_at, id);

ALTER TABLE affiliate_transfers ADD COLUMN balance_after_cents INTEGER;
ALTER TABLE affiliate_transfers ADD COLUMN available_after_cents INTEGER;
ALTER TABLE affiliate_transfers ADD COLUMN frozen_after_cents INTEGER;
ALTER TABLE affiliate_transfers ADD COLUMN history_after_cents INTEGER;
ALTER TABLE affiliate_transfers ADD COLUMN debt_after_cents INTEGER;

INSERT OR IGNORE INTO app_settings (key, value) VALUES
    ('affiliate_rebate_freeze_hours', '0'),
    ('affiliate_rebate_duration_days', '0'),
    ('affiliate_rebate_per_invitee_cap_cents', '0');
