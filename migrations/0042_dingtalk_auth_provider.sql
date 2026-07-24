INSERT OR IGNORE INTO external_auth_providers (
    provider, name, authorize_url, token_url, userinfo_url, scopes,
    subject_path, email_path, display_name_path, token_auth_method, use_pkce, profile_mode
) VALUES (
    'dingtalk', 'DingTalk', 'https://login.dingtalk.com/oauth2/auth',
    'https://api.dingtalk.com/v1.0/oauth2/userAccessToken',
    'https://api.dingtalk.com/v1.0/contact/users/me', 'openid',
    'unionId', 'email', 'nick', 'client_secret_post', 0, 'oidc'
);
