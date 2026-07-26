CREATE TABLE tls_fingerprint_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    enable_grease INTEGER NOT NULL DEFAULT 0 CHECK (enable_grease IN (0, 1)),
    cipher_suites TEXT NOT NULL DEFAULT '[]',
    curves TEXT NOT NULL DEFAULT '[]',
    point_formats TEXT NOT NULL DEFAULT '[]',
    signature_algorithms TEXT NOT NULL DEFAULT '[]',
    alpn_protocols TEXT NOT NULL DEFAULT '[]',
    supported_versions TEXT NOT NULL DEFAULT '[]',
    key_share_groups TEXT NOT NULL DEFAULT '[]',
    psk_modes TEXT NOT NULL DEFAULT '[]',
    extensions TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE accounts ADD COLUMN notes TEXT NOT NULL DEFAULT '';
ALTER TABLE accounts ADD COLUMN crs_account_id TEXT;
ALTER TABLE accounts ADD COLUMN tls_fingerprint_profile_id INTEGER
    REFERENCES tls_fingerprint_profiles(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX idx_accounts_crs_account
    ON accounts(crs_account_id)
    WHERE crs_account_id IS NOT NULL;
CREATE INDEX idx_accounts_tls_fingerprint
    ON accounts(tls_fingerprint_profile_id);

CREATE TABLE error_passthrough_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    priority INTEGER NOT NULL DEFAULT 0,
    error_codes TEXT NOT NULL DEFAULT '[]',
    keywords TEXT NOT NULL DEFAULT '[]',
    match_mode TEXT NOT NULL DEFAULT 'any' CHECK (match_mode IN ('any', 'all')),
    platforms TEXT NOT NULL DEFAULT '["openai"]',
    passthrough_code INTEGER NOT NULL DEFAULT 1 CHECK (passthrough_code IN (0, 1)),
    response_code INTEGER,
    passthrough_body INTEGER NOT NULL DEFAULT 1 CHECK (passthrough_body IN (0, 1)),
    custom_message TEXT,
    skip_monitoring INTEGER NOT NULL DEFAULT 0 CHECK (skip_monitoring IN (0, 1)),
    description TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_error_passthrough_order
    ON error_passthrough_rules(enabled, priority, id);
