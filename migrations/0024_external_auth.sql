CREATE TABLE IF NOT EXISTS external_auth_providers (
    provider TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    client_id TEXT NOT NULL DEFAULT '',
    encrypted_client_secret TEXT,
    authorize_url TEXT NOT NULL DEFAULT '',
    token_url TEXT NOT NULL DEFAULT '',
    userinfo_url TEXT NOT NULL DEFAULT '',
    scopes TEXT NOT NULL DEFAULT 'openid email profile',
    subject_path TEXT NOT NULL DEFAULT 'sub',
    email_path TEXT NOT NULL DEFAULT 'email',
    display_name_path TEXT NOT NULL DEFAULT 'name',
    token_auth_method TEXT NOT NULL DEFAULT 'client_secret_post'
        CHECK (token_auth_method IN ('client_secret_post', 'client_secret_basic', 'none')),
    use_pkce INTEGER NOT NULL DEFAULT 1 CHECK (use_pkce IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT OR IGNORE INTO external_auth_providers (provider, name)
VALUES ('oidc', 'OIDC');

CREATE TABLE IF NOT EXISTS external_auth_identities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL REFERENCES external_auth_providers(provider) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    display_name TEXT,
    email TEXT COLLATE NOCASE,
    email_verified INTEGER NOT NULL DEFAULT 0 CHECK (email_verified IN (0, 1)),
    last_login_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(provider, subject),
    UNIQUE(user_id, provider)
);

CREATE INDEX IF NOT EXISTS idx_external_auth_identities_user
ON external_auth_identities(user_id, provider);

CREATE TABLE IF NOT EXISTS external_oauth_flows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL REFERENCES external_auth_providers(provider) ON DELETE CASCADE,
    state_hash TEXT NOT NULL UNIQUE,
    intent TEXT NOT NULL CHECK (intent IN ('login', 'bind')),
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    encrypted_verifier TEXT,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((intent = 'bind' AND user_id IS NOT NULL) OR (intent = 'login' AND user_id IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_external_oauth_flows_expiry
ON external_oauth_flows(expires_at, consumed_at);
