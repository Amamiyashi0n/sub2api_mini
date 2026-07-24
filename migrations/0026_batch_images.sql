ALTER TABLE users ADD COLUMN frozen_balance_cents INTEGER NOT NULL DEFAULT 0
    CHECK (frozen_balance_cents >= 0);

CREATE TABLE batch_image_providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    kind TEXT NOT NULL DEFAULT 'gemini_api' CHECK (kind IN ('gemini_api')),
    base_url TEXT NOT NULL DEFAULT 'https://generativelanguage.googleapis.com',
    encrypted_api_key TEXT NOT NULL,
    models TEXT NOT NULL DEFAULT '[]',
    unit_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (unit_price_cents >= 0),
    batch_discount_bps INTEGER NOT NULL DEFAULT 5000 CHECK (batch_discount_bps BETWEEN 0 AND 10000),
    hold_bps INTEGER NOT NULL DEFAULT 6000 CHECK (hold_bps BETWEEN 0 AND 10000),
    priority INTEGER NOT NULL DEFAULT 50 CHECK (priority >= 0),
    concurrency INTEGER NOT NULL DEFAULT 1 CHECK (concurrency BETWEEN 1 AND 16),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    last_used_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_batch_image_providers_schedule
    ON batch_image_providers(enabled, priority, id);

CREATE TABLE batch_image_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    api_key_id INTEGER REFERENCES api_keys(id) ON DELETE SET NULL,
    provider_id INTEGER NOT NULL REFERENCES batch_image_providers(id) ON DELETE RESTRICT,
    task_name TEXT NOT NULL DEFAULT '',
    parent_batch_id TEXT,
    status TEXT NOT NULL DEFAULT 'created' CHECK (status IN (
        'created', 'queued', 'running', 'indexing', 'settling',
        'completed', 'failed', 'cancelled', 'output_deleted'
    )),
    model TEXT NOT NULL,
    response_mime_type TEXT NOT NULL DEFAULT 'image/png',
    image_size TEXT NOT NULL DEFAULT '1K',
    item_count INTEGER NOT NULL CHECK (item_count > 0),
    requested_image_count INTEGER NOT NULL CHECK (requested_image_count > 0),
    success_count INTEGER NOT NULL DEFAULT 0 CHECK (success_count >= 0),
    fail_count INTEGER NOT NULL DEFAULT 0 CHECK (fail_count >= 0),
    generated_image_count INTEGER NOT NULL DEFAULT 0 CHECK (generated_image_count >= 0),
    estimated_cost_cents INTEGER NOT NULL DEFAULT 0 CHECK (estimated_cost_cents >= 0),
    hold_amount_cents INTEGER NOT NULL DEFAULT 0 CHECK (hold_amount_cents >= 0),
    billable_unit_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (billable_unit_price_cents >= 0),
    hold_unit_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (hold_unit_price_cents >= 0),
    actual_cost_cents INTEGER CHECK (actual_cost_cents >= 0),
    provider_job_name TEXT,
    provider_input_ref TEXT,
    provider_output_ref TEXT,
    idempotency_key TEXT,
    request_hash TEXT NOT NULL,
    last_error_code TEXT,
    last_error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    output_expires_at TEXT,
    downloaded_at TEXT,
    output_deleted_at TEXT,
    user_deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    submitted_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    settled_at TEXT
);

CREATE INDEX idx_batch_image_jobs_owner
    ON batch_image_jobs(user_id, api_key_id, created_at DESC, id DESC);
CREATE INDEX idx_batch_image_jobs_status
    ON batch_image_jobs(status, updated_at, id);
CREATE INDEX idx_batch_image_jobs_parent
    ON batch_image_jobs(parent_batch_id, created_at);
CREATE UNIQUE INDEX idx_batch_image_jobs_idempotency
    ON batch_image_jobs(api_key_id, idempotency_key)
    WHERE api_key_id IS NOT NULL AND idempotency_key IS NOT NULL AND idempotency_key <> '';

CREATE TABLE batch_image_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES batch_image_jobs(id) ON DELETE CASCADE,
    custom_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'success', 'failed', 'cancelled')),
    output_count INTEGER NOT NULL DEFAULT 1 CHECK (output_count BETWEEN 1 AND 4),
    prompt_hash TEXT NOT NULL,
    mime_type TEXT,
    file_extension TEXT,
    image_count INTEGER NOT NULL DEFAULT 0 CHECK (image_count >= 0),
    output_files TEXT NOT NULL DEFAULT '[]',
    error_code TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    indexed_at TEXT,
    UNIQUE(job_id, custom_id)
);

CREATE INDEX idx_batch_image_items_job_status
    ON batch_image_items(job_id, status, id);

CREATE TABLE batch_image_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES batch_image_jobs(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_batch_image_events_job
    ON batch_image_events(job_id, created_at, id);
