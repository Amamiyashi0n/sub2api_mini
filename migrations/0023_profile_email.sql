CREATE TABLE IF NOT EXISTS profile_email_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email TEXT NOT NULL COLLATE NOCASE,
    code_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_profile_email_changes_user
ON profile_email_changes(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_profile_email_changes_email
ON profile_email_changes(email, created_at DESC);
