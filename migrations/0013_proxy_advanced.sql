ALTER TABLE proxies ADD COLUMN fallback_mode TEXT NOT NULL DEFAULT 'none'
    CHECK (fallback_mode IN ('none', 'proxy', 'direct'));
ALTER TABLE proxies ADD COLUMN backup_proxy_id INTEGER REFERENCES proxies(id) ON DELETE SET NULL;
ALTER TABLE proxies ADD COLUMN expiry_warn_days INTEGER NOT NULL DEFAULT 7 CHECK (expiry_warn_days >= 0);
ALTER TABLE proxies ADD COLUMN last_ip_address TEXT;
ALTER TABLE proxies ADD COLUMN last_country TEXT;
ALTER TABLE proxies ADD COLUMN last_country_code TEXT;
ALTER TABLE proxies ADD COLUMN last_region TEXT;
ALTER TABLE proxies ADD COLUMN last_city TEXT;
ALTER TABLE proxies ADD COLUMN quality_score INTEGER;
ALTER TABLE proxies ADD COLUMN quality_grade TEXT;
ALTER TABLE proxies ADD COLUMN quality_summary TEXT;
ALTER TABLE proxies ADD COLUMN quality_checked_at TEXT;

CREATE INDEX IF NOT EXISTS idx_proxies_backup ON proxies(backup_proxy_id);
