ALTER TABLE prompt_audit_jobs ADD COLUMN queue_delay_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE prompt_audit_jobs ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_prompt_audit_jobs_processed
    ON prompt_audit_jobs(processed_at DESC, id DESC);
