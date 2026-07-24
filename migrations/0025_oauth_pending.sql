CREATE TABLE IF NOT EXISTS external_oauth_pending (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL REFERENCES external_auth_providers(provider) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    encrypted_profile TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_external_oauth_pending_expiry
ON external_oauth_pending(expires_at, consumed_at);
