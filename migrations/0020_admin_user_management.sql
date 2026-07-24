ALTER TABLE users ADD COLUMN notes TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN deleted_at TEXT;

CREATE INDEX IF NOT EXISTS idx_users_active_role
    ON users(deleted_at, role, enabled, id);

CREATE TABLE IF NOT EXISTS user_balance_adjustments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    admin_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    delta_cents INTEGER NOT NULL CHECK (delta_cents != 0),
    balance_after_cents INTEGER NOT NULL CHECK (balance_after_cents >= 0),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_balance_adjustments_user
    ON user_balance_adjustments(user_id, created_at DESC, id DESC);
