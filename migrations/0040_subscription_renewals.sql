ALTER TABLE subscriptions ADD COLUMN auto_renew INTEGER NOT NULL DEFAULT 0
    CHECK (auto_renew IN (0, 1));
ALTER TABLE subscriptions ADD COLUMN renewal_status TEXT NOT NULL DEFAULT 'disabled'
    CHECK (renewal_status IN ('disabled', 'scheduled', 'succeeded', 'insufficient_balance', 'plan_unavailable', 'error'));
ALTER TABLE subscriptions ADD COLUMN next_renewal_at TEXT;
ALTER TABLE subscriptions ADD COLUMN last_renewal_at TEXT;
ALTER TABLE subscriptions ADD COLUMN last_renewal_error TEXT NOT NULL DEFAULT '';

ALTER TABLE orders ADD COLUMN order_type TEXT NOT NULL DEFAULT 'purchase'
    CHECK (order_type IN ('purchase', 'renewal'));
ALTER TABLE orders ADD COLUMN renewal_key TEXT;
CREATE UNIQUE INDEX idx_orders_renewal_key ON orders(renewal_key) WHERE renewal_key IS NOT NULL;

CREATE TABLE subscription_renewal_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subscription_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    period_end TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('processing', 'succeeded', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 1,
    order_id INTEGER REFERENCES orders(id) ON DELETE SET NULL,
    amount_cents INTEGER NOT NULL DEFAULT 0,
    error_code TEXT NOT NULL DEFAULT '',
    attempted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(subscription_id, period_end)
);

CREATE INDEX idx_subscriptions_auto_renew_due
    ON subscriptions(auto_renew, next_renewal_at, status);
CREATE INDEX idx_subscription_renewal_attempts_subscription
    ON subscription_renewal_attempts(subscription_id, attempted_at DESC);
