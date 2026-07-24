ALTER TABLE batch_image_providers ADD COLUMN provider_type TEXT NOT NULL DEFAULT 'gemini_api'
    CHECK (provider_type IN ('gemini_api', 'vertex'));
ALTER TABLE batch_image_providers ADD COLUMN encrypted_service_account_json TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_providers ADD COLUMN project_id TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_providers ADD COLUMN location TEXT NOT NULL DEFAULT 'global';
ALTER TABLE batch_image_providers ADD COLUMN gcs_bucket TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_providers ADD COLUMN gcs_prefix TEXT NOT NULL DEFAULT 'batch-image/mini/{batch_id}';
ALTER TABLE batch_image_providers ADD COLUMN gcs_base_url TEXT NOT NULL DEFAULT 'https://storage.googleapis.com';
ALTER TABLE batch_image_providers ADD COLUMN token_url TEXT NOT NULL DEFAULT 'https://oauth2.googleapis.com/token';

ALTER TABLE batch_image_jobs ADD COLUMN provider_kind TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_base_url_snapshot TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_project_id TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_location TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_gcs_bucket TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_gcs_prefix TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_gcs_base_url TEXT NOT NULL DEFAULT '';
ALTER TABLE batch_image_jobs ADD COLUMN provider_token_url TEXT NOT NULL DEFAULT '';

UPDATE batch_image_jobs SET
    provider_kind = COALESCE((SELECT provider_type FROM batch_image_providers WHERE id = provider_id), 'gemini_api'),
    provider_base_url_snapshot = COALESCE((SELECT base_url FROM batch_image_providers WHERE id = provider_id), ''),
    provider_project_id = COALESCE((SELECT project_id FROM batch_image_providers WHERE id = provider_id), ''),
    provider_location = COALESCE((SELECT location FROM batch_image_providers WHERE id = provider_id), 'global'),
    provider_gcs_bucket = COALESCE((SELECT gcs_bucket FROM batch_image_providers WHERE id = provider_id), ''),
    provider_gcs_prefix = COALESCE((SELECT gcs_prefix FROM batch_image_providers WHERE id = provider_id), ''),
    provider_gcs_base_url = COALESCE((SELECT gcs_base_url FROM batch_image_providers WHERE id = provider_id), ''),
    provider_token_url = COALESCE((SELECT token_url FROM batch_image_providers WHERE id = provider_id), '');

DROP INDEX idx_batch_image_providers_schedule;
CREATE INDEX idx_batch_image_providers_schedule
    ON batch_image_providers(enabled, provider_type, priority, id);
