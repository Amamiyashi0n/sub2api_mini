ALTER TABLE users ADD COLUMN allow_all_standard_groups INTEGER NOT NULL DEFAULT 1
    CHECK (allow_all_standard_groups IN (0, 1));

CREATE TABLE user_allowed_groups (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, group_id)
);

CREATE INDEX idx_user_allowed_groups_group
    ON user_allowed_groups(group_id, user_id);

ALTER TABLE plans ADD COLUMN group_id INTEGER REFERENCES groups(id) ON DELETE RESTRICT;
CREATE INDEX idx_plans_group ON plans(group_id, enabled, sort_order);

ALTER TABLE subscriptions ADD COLUMN group_id INTEGER REFERENCES groups(id) ON DELETE RESTRICT;
UPDATE subscriptions
SET group_id = (SELECT plans.group_id FROM plans WHERE plans.id = subscriptions.plan_id)
WHERE group_id IS NULL;

DROP INDEX idx_subscriptions_one_active;
CREATE UNIQUE INDEX idx_subscriptions_one_active_global
    ON subscriptions(user_id) WHERE status = 'active' AND group_id IS NULL;
CREATE UNIQUE INDEX idx_subscriptions_one_active_group
    ON subscriptions(user_id, group_id)
    WHERE status = 'active' AND group_id IS NOT NULL;
CREATE INDEX idx_subscriptions_group
    ON subscriptions(group_id, status, ends_at, user_id);
