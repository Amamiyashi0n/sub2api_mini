ALTER TABLE external_auth_providers ADD COLUMN issuer_url TEXT NOT NULL DEFAULT '';
ALTER TABLE external_auth_providers ADD COLUMN discovery_url TEXT NOT NULL DEFAULT '';
ALTER TABLE external_auth_providers ADD COLUMN jwks_url TEXT NOT NULL DEFAULT '';
ALTER TABLE external_auth_providers ADD COLUMN validate_id_token INTEGER NOT NULL DEFAULT 0
    CHECK (validate_id_token IN (0, 1));
ALTER TABLE external_auth_providers ADD COLUMN allowed_signing_algs TEXT NOT NULL DEFAULT 'RS256,ES256,PS256';
ALTER TABLE external_auth_providers ADD COLUMN clock_skew_seconds INTEGER NOT NULL DEFAULT 120
    CHECK (clock_skew_seconds BETWEEN 0 AND 600);
ALTER TABLE external_auth_providers ADD COLUMN require_email_verified INTEGER NOT NULL DEFAULT 0
    CHECK (require_email_verified IN (0, 1));

ALTER TABLE external_oauth_flows ADD COLUMN encrypted_nonce TEXT;
