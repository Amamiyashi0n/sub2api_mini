ALTER TABLE plans ADD COLUMN original_price_cents INTEGER NOT NULL DEFAULT 0
    CHECK (original_price_cents >= 0);
ALTER TABLE plans ADD COLUMN currency TEXT NOT NULL DEFAULT 'CNY'
    CHECK (length(currency) = 3);
ALTER TABLE plans ADD COLUMN features TEXT NOT NULL DEFAULT '[]';
ALTER TABLE plans ADD COLUMN product_name TEXT NOT NULL DEFAULT '';

ALTER TABLE payment_orders ADD COLUMN fulfillment_type TEXT NOT NULL DEFAULT 'purchase'
    CHECK (fulfillment_type IN ('purchase', 'renewal'));
ALTER TABLE payment_orders ADD COLUMN renewal_previous_status TEXT;
ALTER TABLE payment_orders ADD COLUMN renewal_previous_starts_at TEXT;
ALTER TABLE payment_orders ADD COLUMN renewal_previous_ends_at TEXT;
ALTER TABLE payment_orders ADD COLUMN renewal_previous_token_limit INTEGER;
ALTER TABLE payment_orders ADD COLUMN renewal_result_ends_at TEXT;
ALTER TABLE payment_orders ADD COLUMN renewal_added_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE payment_orders ADD COLUMN renewal_added_tokens INTEGER NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX idx_payment_orders_pending_subscription_renewal
ON payment_orders(subscription_id)
WHERE fulfillment_type = 'renewal' AND status IN ('PENDING', 'PAID', 'RECHARGING');
