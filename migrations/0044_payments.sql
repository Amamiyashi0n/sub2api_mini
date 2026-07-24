CREATE TABLE payment_provider_instances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_key TEXT NOT NULL CHECK (provider_key IN ('stripe', 'airwallex', 'alipay', 'wxpay', 'easypay')),
    name TEXT NOT NULL,
    encrypted_config TEXT NOT NULL,
    supported_types TEXT NOT NULL DEFAULT 'card',
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    payment_mode TEXT NOT NULL DEFAULT 'popup' CHECK (payment_mode IN ('popup', 'redirect', 'qrcode')),
    sort_order INTEGER NOT NULL DEFAULT 0,
    min_amount_cents INTEGER NOT NULL DEFAULT 100 CHECK (min_amount_cents > 0),
    max_amount_cents INTEGER NOT NULL DEFAULT 10000000 CHECK (max_amount_cents >= min_amount_cents),
    refund_enabled INTEGER NOT NULL DEFAULT 0 CHECK (refund_enabled IN (0, 1)),
    allow_user_refund INTEGER NOT NULL DEFAULT 0 CHECK (allow_user_refund IN (0, 1)),
    deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_payment_providers_available
    ON payment_provider_instances(provider_key, enabled, deleted_at, sort_order, id);

CREATE TABLE payment_orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    provider_instance_id INTEGER REFERENCES payment_provider_instances(id) ON DELETE SET NULL,
    provider_key TEXT NOT NULL,
    payment_type TEXT NOT NULL,
    order_type TEXT NOT NULL CHECK (order_type IN ('balance', 'subscription')),
    plan_id INTEGER REFERENCES plans(id) ON DELETE RESTRICT,
    plan_name TEXT,
    plan_token_limit INTEGER CHECK (plan_token_limit IS NULL OR plan_token_limit >= 0),
    plan_duration_days INTEGER CHECK (plan_duration_days IS NULL OR plan_duration_days > 0),
    plan_group_id INTEGER REFERENCES groups(id) ON DELETE RESTRICT,
    subscription_id INTEGER REFERENCES subscriptions(id) ON DELETE SET NULL,
    internal_order_id INTEGER UNIQUE REFERENCES orders(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN (
        'PENDING', 'PAID', 'RECHARGING', 'COMPLETED', 'EXPIRED', 'CANCELLED', 'FAILED',
        'REFUND_REQUESTED', 'REFUNDING', 'REFUND_PENDING', 'PARTIALLY_REFUNDED',
        'REFUNDED', 'REFUND_FAILED'
    )),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    credit_cents INTEGER NOT NULL DEFAULT 0 CHECK (credit_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (length(currency) = 3),
    out_trade_no TEXT NOT NULL UNIQUE,
    payment_trade_no TEXT,
    idempotency_key TEXT,
    pay_url TEXT,
    qr_code TEXT,
    encrypted_client_secret TEXT,
    provider_snapshot TEXT NOT NULL DEFAULT '{}',
    source TEXT NOT NULL DEFAULT 'web',
    expires_at TEXT NOT NULL,
    paid_at TEXT,
    completed_at TEXT,
    cancelled_at TEXT,
    failed_at TEXT,
    failure_code TEXT NOT NULL DEFAULT '',
    failure_reason TEXT NOT NULL DEFAULT '',
    refunded_cents INTEGER NOT NULL DEFAULT 0 CHECK (refunded_cents >= 0),
    refund_status TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((order_type = 'subscription' AND plan_id IS NOT NULL AND plan_name IS NOT NULL
            AND plan_token_limit IS NOT NULL AND plan_duration_days IS NOT NULL AND credit_cents = 0)
        OR (order_type = 'balance' AND plan_id IS NULL AND credit_cents > 0))
);

CREATE UNIQUE INDEX idx_payment_orders_user_idempotency
    ON payment_orders(user_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE UNIQUE INDEX idx_payment_orders_trade_no
    ON payment_orders(provider_instance_id, payment_trade_no) WHERE payment_trade_no IS NOT NULL;
CREATE INDEX idx_payment_orders_user
    ON payment_orders(user_id, created_at DESC, id DESC);
CREATE INDEX idx_payment_orders_status
    ON payment_orders(status, expires_at, id);
CREATE INDEX idx_payment_orders_provider
    ON payment_orders(provider_instance_id, created_at DESC, id DESC);

CREATE TABLE payment_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_instance_id INTEGER REFERENCES payment_provider_instances(id) ON DELETE SET NULL,
    provider_key TEXT NOT NULL,
    event_id TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    payment_order_id INTEGER REFERENCES payment_orders(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'processing' CHECK (status IN ('processing', 'processed', 'ignored', 'failed')),
    error_summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processed_at TEXT,
    UNIQUE(provider_key, event_id)
);

CREATE INDEX idx_payment_events_order
    ON payment_events(payment_order_id, created_at DESC);

CREATE TABLE payment_refunds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payment_order_id INTEGER NOT NULL REFERENCES payment_orders(id) ON DELETE RESTRICT,
    provider_refund_id TEXT,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    status TEXT NOT NULL DEFAULT 'REQUESTED' CHECK (status IN ('REQUESTED', 'PROCESSING', 'PENDING', 'COMPLETED', 'FAILED', 'CANCELLED')),
    reason TEXT NOT NULL DEFAULT '',
    requested_by_user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    failure_reason TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT
);

CREATE INDEX idx_payment_refunds_order
    ON payment_refunds(payment_order_id, created_at DESC);
CREATE UNIQUE INDEX idx_payment_refunds_active
    ON payment_refunds(payment_order_id)
    WHERE status IN ('REQUESTED', 'PROCESSING', 'PENDING');

INSERT OR IGNORE INTO app_settings (key, value) VALUES
    ('payment_enabled', 'true'),
    ('payment_order_expiry_minutes', '30');
