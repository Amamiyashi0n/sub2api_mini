PRAGMA writable_schema = ON;

UPDATE sqlite_schema
SET sql = replace(
    replace(
        replace(
            sql,
            'kind TEXT NOT NULL CHECK (kind IN (''api_key'', ''oauth''))',
            'kind TEXT NOT NULL CHECK (kind IN (''api_key'', ''oauth'', ''bedrock'', ''service_account''))'
        ),
        'CHECK (platform IN (''openai'', ''anthropic''))',
        'CHECK (platform IN (''openai'', ''anthropic'', ''gemini'', ''antigravity'', ''grok''))'
    ),
    'CHECK (account_type IN (''api_key'', ''oauth'', ''setup_token''))',
    'CHECK (account_type IN (''api_key'', ''oauth'', ''setup_token'', ''upstream'', ''bedrock'', ''service_account''))'
)
WHERE type = 'table' AND name = 'accounts';

PRAGMA writable_schema = OFF;
PRAGMA schema_version = 10058;
