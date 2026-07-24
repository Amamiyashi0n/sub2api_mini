CREATE TABLE channel_monitor_templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('openai', 'anthropic', 'gemini', 'grok')),
    api_mode TEXT NOT NULL DEFAULT 'chat_completions'
        CHECK (api_mode IN ('chat_completions', 'responses')),
    description TEXT NOT NULL DEFAULT '',
    encrypted_template_config TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (provider, api_mode, name)
);

ALTER TABLE channel_monitors ADD COLUMN template_id INTEGER
    REFERENCES channel_monitor_templates(id) ON DELETE SET NULL;

CREATE INDEX idx_channel_monitor_templates_provider
    ON channel_monitor_templates(provider, api_mode, name);
CREATE INDEX idx_channel_monitors_template
    ON channel_monitors(template_id, id);
