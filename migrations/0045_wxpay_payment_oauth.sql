CREATE TABLE wxpay_payment_oauth_flows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_instance_id INTEGER NOT NULL REFERENCES payment_provider_instances(id) ON DELETE CASCADE,
    state_hash TEXT NOT NULL UNIQUE,
    resume_token_hash TEXT UNIQUE,
    encrypted_openid TEXT,
    return_hash TEXT NOT NULL DEFAULT '#/purchase',
    expires_at TEXT NOT NULL,
    callback_consumed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_wxpay_payment_oauth_expiry
    ON wxpay_payment_oauth_flows(expires_at, callback_consumed_at);
