INSERT OR IGNORE INTO external_auth_providers (
    provider, name, authorize_url, token_url, userinfo_url, scopes,
    subject_path, email_path, display_name_path, token_auth_method, use_pkce, profile_mode
) VALUES
    ('wechat_open', 'WeChat Open', 'https://open.weixin.qq.com/connect/qrconnect',
     'https://api.weixin.qq.com/sns/oauth2/access_token',
     'https://api.weixin.qq.com/sns/userinfo', 'snsapi_login',
     'unionid', 'email', 'nickname', 'client_secret_post', 0, 'oidc'),
    ('wechat_mp', 'WeChat MP', 'https://open.weixin.qq.com/connect/oauth2/authorize',
     'https://api.weixin.qq.com/sns/oauth2/access_token',
     'https://api.weixin.qq.com/sns/userinfo', 'snsapi_userinfo',
     'unionid', 'email', 'nickname', 'client_secret_post', 0, 'oidc');
