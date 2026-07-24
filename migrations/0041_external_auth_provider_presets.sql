ALTER TABLE external_auth_providers ADD COLUMN profile_mode TEXT NOT NULL DEFAULT 'oidc'
    CHECK (profile_mode IN ('oidc', 'github', 'google', 'linuxdo'));
ALTER TABLE external_auth_providers ADD COLUMN emails_url TEXT NOT NULL DEFAULT '';

INSERT OR IGNORE INTO external_auth_providers (
    provider, name, authorize_url, token_url, userinfo_url, emails_url, scopes,
    subject_path, email_path, display_name_path, token_auth_method, use_pkce, profile_mode
) VALUES
    ('github', 'GitHub', 'https://github.com/login/oauth/authorize',
     'https://github.com/login/oauth/access_token', 'https://api.github.com/user',
     'https://api.github.com/user/emails', 'read:user user:email', 'id', 'email',
     'name', 'client_secret_post', 0, 'github'),
    ('google', 'Google', 'https://accounts.google.com/o/oauth2/v2/auth',
     'https://oauth2.googleapis.com/token', 'https://openidconnect.googleapis.com/v1/userinfo',
     '', 'openid email profile', 'sub', 'email', 'name', 'client_secret_post', 1, 'google'),
    ('linuxdo', 'LinuxDo', 'https://connect.linux.do/oauth2/authorize',
     'https://connect.linux.do/oauth2/token', 'https://connect.linux.do/api/user',
     '', 'user', 'id', 'email', 'name', 'client_secret_post', 0, 'linuxdo');
