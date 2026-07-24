ALTER TABLE accounts ADD COLUMN parent_account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE accounts ADD COLUMN quota_dimension TEXT NOT NULL DEFAULT 'global'
    CHECK (quota_dimension IN ('global', 'spark'));

CREATE UNIQUE INDEX idx_accounts_single_spark_shadow
    ON accounts(parent_account_id)
    WHERE parent_account_id IS NOT NULL;
CREATE INDEX idx_accounts_parent ON accounts(parent_account_id);
