ALTER TABLE external_auth_providers ADD COLUMN dingtalk_app_type TEXT NOT NULL DEFAULT 'public'
    CHECK (dingtalk_app_type IN ('public', 'internal'));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_corp_policy TEXT NOT NULL DEFAULT 'none'
    CHECK (dingtalk_corp_policy IN ('none', 'internal_only'));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_internal_corp_id TEXT NOT NULL DEFAULT '';
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_bypass_registration INTEGER NOT NULL DEFAULT 0
    CHECK (dingtalk_bypass_registration IN (0, 1));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_sync_corp_email INTEGER NOT NULL DEFAULT 0
    CHECK (dingtalk_sync_corp_email IN (0, 1));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_sync_display_name INTEGER NOT NULL DEFAULT 0
    CHECK (dingtalk_sync_display_name IN (0, 1));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_sync_dept INTEGER NOT NULL DEFAULT 0
    CHECK (dingtalk_sync_dept IN (0, 1));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_require_email INTEGER NOT NULL DEFAULT 0
    CHECK (dingtalk_require_email IN (0, 1));
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_email_attr_key TEXT NOT NULL DEFAULT 'dingtalk_email';
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_email_attr_name TEXT NOT NULL DEFAULT 'DingTalk corporate email';
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_name_attr_key TEXT NOT NULL DEFAULT 'dingtalk_name';
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_name_attr_name TEXT NOT NULL DEFAULT 'DingTalk display name';
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_dept_attr_key TEXT NOT NULL DEFAULT 'dingtalk_department';
ALTER TABLE external_auth_providers ADD COLUMN dingtalk_dept_attr_name TEXT NOT NULL DEFAULT 'DingTalk department';

CREATE TABLE IF NOT EXISTS user_external_attributes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL REFERENCES external_auth_providers(provider) ON DELETE CASCADE,
    attribute_key TEXT NOT NULL,
    attribute_name TEXT NOT NULL,
    value TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, provider, attribute_key)
);

CREATE INDEX IF NOT EXISTS idx_user_external_attributes_user
ON user_external_attributes(user_id, provider);
