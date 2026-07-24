"use strict";

window.Sub2MiniIdentity = (() => {
  function profileRows(data) {
    const identities = new Map(data.identities.map(item => [item.provider, item]));
    const providers = data.providers.filter(provider => provider.enabled || identities.has(provider.provider));
    if (!providers.length) return '<p class="field-hint">管理员尚未启用第三方身份提供商。</p>';
    return `<div class="identity-list">${providers.map(provider => {
      const identity = provider.provider.startsWith("wechat_") ? (identities.get("wechat_open") || identities.get("wechat_mp")) : identities.get(provider.provider);
      return `<div class="identity-row"><div><strong>${escapeHtml(provider.name)}</strong><span>${identity ? `${escapeHtml(identity.display_name || identity.email || "已绑定")} · ${escapeHtml(identity.subject_hint)}` : "未绑定"}</span></div>${identity ? `<button class="button danger small" data-identity-action="unbind" data-provider="${escapeHtml(provider.provider)}" data-name="${escapeHtml(provider.name)}">解绑</button>` : `<button class="button small" data-identity-action="bind" data-provider="${escapeHtml(provider.provider)}">绑定</button>`}</div>`;
    }).join("")}</div>`;
  }

  function attachProfile(page) {
    page.querySelectorAll("[data-identity-action]").forEach(button => button.addEventListener("click", handleIdentityAction));
  }

  async function handleIdentityAction(event) {
    const button = event.currentTarget;
    const provider = button.dataset.provider;
    if (button.dataset.identityAction === "bind") {
      button.disabled = true;
      try {
        const result = await api(`/api/user/auth-identities/${encodeURIComponent(provider)}/start`, { method: "POST", body: "{}" });
        location.href = result.data.authorization_url;
      } catch (error) { toast(error.message, true); button.disabled = false; }
      return;
    }
    openModal("解绑第三方身份", `<p>确认解绑 <strong>${escapeHtml(button.dataset.name)}</strong>？其他设备会话将退出。</p><p class="form-error" id="identity-unbind-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-identity-unbind">解绑</button>');
    modal.querySelector("#confirm-identity-unbind").addEventListener("click", async click => {
      click.currentTarget.disabled = true;
      try { await api(`/api/user/auth-identities/${encodeURIComponent(provider)}`, { method: "DELETE" }); closeModal(); toast("第三方身份已解绑"); await renderRoute(); }
      catch (error) { modal.querySelector("#identity-unbind-error").textContent = error.message; click.currentTarget.disabled = false; }
    });
  }

  function settingsPanel(providers) {
    return providers.map(providerSettingsPanel).join("");
  }

  function providerSettingsPanel(provider) {
    const key = escapeHtml(provider.provider);
    const description = provider.provider === "wechat_open" ? "使用微信开放平台扫码授权。" : provider.provider === "wechat_mp" ? "使用微信公众号网页授权。" : provider.provider === "dingtalk" ? "支持个人授权、企业内部成员校验和组织资料同步。" : provider.profile_mode === "github" ? "读取 GitHub 已验证邮箱后绑定本地账户。" : provider.profile_mode === "google" ? "要求 Google 返回已验证邮箱。" : provider.profile_mode === "linuxdo" ? "使用 LinuxDo Connect，身份不会按邮箱自动合并。" : "使用 Authorization Code、PKCE 和 UserInfo 绑定本地账户。";
    const oidcSecurity = provider.provider === "oidc" ? `
        <div class="field"><label>Discovery URL（可选）</label><input name="discovery_url" type="url" value="${escapeHtml(provider.discovery_url || "")}" placeholder="https://issuer.example/.well-known/openid-configuration"><span class="field-hint">保存时自动补全空白端点，并校验 issuer 一致性</span></div>
        <div class="form-grid"><div class="field"><label>Issuer URL</label><input name="issuer_url" type="url" value="${escapeHtml(provider.issuer_url || "")}"></div><div class="field"><label>JWKS URL</label><input name="jwks_url" type="url" value="${escapeHtml(provider.jwks_url || "")}"></div></div>
        <label class="toggle-line"><input data-role="validate-id-token" type="checkbox" ${provider.validate_id_token ? "checked" : ""}> 验证签名 ID Token、nonce、issuer、audience 和有效期</label>
        <div class="form-grid"><div class="field"><label>允许的签名算法</label><input name="allowed_signing_algs" value="${escapeHtml(provider.allowed_signing_algs || "RS256,ES256,PS256")}" required></div><div class="field"><label>时钟偏差（秒）</label><input name="clock_skew_seconds" type="number" min="0" max="600" value="${Number(provider.clock_skew_seconds ?? 120)}" required></div></div>
        <label class="toggle-line"><input data-role="require-email-verified" type="checkbox" ${provider.require_email_verified ? "checked" : ""}> 要求 UserInfo 返回已验证邮箱</label>` : '<input name="issuer_url" type="hidden" value=""><input name="discovery_url" type="hidden" value=""><input name="jwks_url" type="hidden" value=""><input name="allowed_signing_algs" type="hidden" value="RS256,ES256,PS256"><input name="clock_skew_seconds" type="hidden" value="120">';
    const dingtalkSecurity = provider.provider === "dingtalk" ? `
        <div class="form-grid"><div class="field"><label>应用类型</label><select name="dingtalk_app_type"><option value="public" ${provider.dingtalk_app_type === "public" ? "selected" : ""}>第三方企业应用</option><option value="internal" ${provider.dingtalk_app_type === "internal" ? "selected" : ""}>企业内部应用</option></select></div><div class="field"><label>企业限制策略</label><select name="dingtalk_corp_policy"><option value="none" ${provider.dingtalk_corp_policy !== "internal_only" ? "selected" : ""}>不限制，跨企业时降级</option><option value="internal_only" ${provider.dingtalk_corp_policy === "internal_only" ? "selected" : ""}>仅内部企业成员</option></select></div></div>
        <div class="field"><label>内部企业 Corp ID（备注，可选）</label><input name="dingtalk_internal_corp_id" value="${escapeHtml(provider.dingtalk_internal_corp_id || "")}" maxlength="256"><span class="field-hint">成员资格由企业应用令牌反查验证，不依赖 OAuth 返回的 Corp ID</span></div>
        <label class="toggle-line"><input data-role="dingtalk-bypass-registration" type="checkbox" ${provider.dingtalk_bypass_registration ? "checked" : ""}> 内部企业成员可在全局关闭注册时创建账户</label>
        <label class="toggle-line"><input data-role="dingtalk-require-email" type="checkbox" ${provider.dingtalk_require_email ? "checked" : ""}> 注册时要求真实邮箱，不使用合成邮箱</label>
        <div class="settings-heading"><h3>企业资料同步</h3><p>仅在“仅内部企业成员”策略下生效。</p></div>
        <label class="toggle-line"><input data-role="dingtalk-sync-email" type="checkbox" ${provider.dingtalk_sync_corp_email ? "checked" : ""}> 同步企业邮箱属性</label>
        <div class="form-grid"><div class="field"><label>邮箱属性 Key</label><input name="dingtalk_email_attr_key" value="${escapeHtml(provider.dingtalk_email_attr_key || "dingtalk_email")}" maxlength="64"></div><div class="field"><label>邮箱属性名称</label><input name="dingtalk_email_attr_name" value="${escapeHtml(provider.dingtalk_email_attr_name || "钉钉企业邮箱")}" maxlength="80"></div></div>
        <label class="toggle-line"><input data-role="dingtalk-sync-name" type="checkbox" ${provider.dingtalk_sync_display_name ? "checked" : ""}> 同步企业显示名属性</label>
        <div class="form-grid"><div class="field"><label>显示名属性 Key</label><input name="dingtalk_name_attr_key" value="${escapeHtml(provider.dingtalk_name_attr_key || "dingtalk_name")}" maxlength="64"></div><div class="field"><label>显示名属性名称</label><input name="dingtalk_name_attr_name" value="${escapeHtml(provider.dingtalk_name_attr_name || "钉钉显示名")}" maxlength="80"></div></div>
        <label class="toggle-line"><input data-role="dingtalk-sync-dept" type="checkbox" ${provider.dingtalk_sync_dept ? "checked" : ""}> 同步企业部门路径属性</label>
        <div class="form-grid"><div class="field"><label>部门属性 Key</label><input name="dingtalk_dept_attr_key" value="${escapeHtml(provider.dingtalk_dept_attr_key || "dingtalk_department")}" maxlength="64"></div><div class="field"><label>部门属性名称</label><input name="dingtalk_dept_attr_name" value="${escapeHtml(provider.dingtalk_dept_attr_name || "钉钉部门")}" maxlength="80"></div></div>` : "";
    return `<section class="settings-panel oidc-settings-panel">
      <div class="settings-heading"><h2>${escapeHtml(provider.name)} 登录</h2><p>${description}</p></div>
      <form class="oauth-provider-form" data-provider="${key}">
        <label class="toggle-line"><input data-role="enabled" type="checkbox" ${provider.enabled ? "checked" : ""}> 启用 ${escapeHtml(provider.name)} 登录</label>
        <div class="field"><label>提供商名称</label><input name="name" value="${escapeHtml(provider.name)}" maxlength="80" required></div>
        <div class="field"><label>Client ID</label><input name="client_id" value="${escapeHtml(provider.client_id)}"></div>
        <div class="field"><label>Client Secret</label><input name="client_secret" type="password" autocomplete="new-password" placeholder="${provider.has_client_secret ? "留空保留已保存密钥" : "未配置"}"></div>
        ${provider.has_client_secret ? '<label class="toggle-line"><input data-role="clear-secret" type="checkbox"> 清除已保存 Client Secret</label>' : ""}
        <div class="field"><label>Authorize URL</label><input name="authorize_url" type="url" value="${escapeHtml(provider.authorize_url)}"></div>
        <div class="field"><label>Token URL</label><input name="token_url" type="url" value="${escapeHtml(provider.token_url)}"></div>
        <div class="field"><label>UserInfo URL</label><input name="userinfo_url" type="url" value="${escapeHtml(provider.userinfo_url)}"></div>
        ${oidcSecurity}
        ${dingtalkSecurity}
        ${provider.profile_mode === "github" ? `<div class="field"><label>Verified Emails URL</label><input name="emails_url" type="url" value="${escapeHtml(provider.emails_url)}"></div>` : '<input name="emails_url" type="hidden" value="">'}
        <div class="field"><label>Scopes</label><input name="scopes" value="${escapeHtml(provider.scopes)}"></div>
        <div class="form-grid"><div class="field"><label>用户 ID 字段</label><input name="subject_path" value="${escapeHtml(provider.subject_path)}" required></div><div class="field"><label>邮箱字段</label><input name="email_path" value="${escapeHtml(provider.email_path)}" required></div></div>
        <div class="form-grid"><div class="field"><label>显示名字段</label><input name="display_name_path" value="${escapeHtml(provider.display_name_path)}" required></div><div class="field"><label>Token 认证</label><select name="token_auth_method"><option value="client_secret_post" ${provider.token_auth_method === "client_secret_post" ? "selected" : ""}>client_secret_post</option><option value="client_secret_basic" ${provider.token_auth_method === "client_secret_basic" ? "selected" : ""}>client_secret_basic</option><option value="none" ${provider.token_auth_method === "none" ? "selected" : ""}>none</option></select></div></div>
        <label class="toggle-line"><input data-role="pkce" type="checkbox" ${provider.use_pkce ? "checked" : ""}> 使用 PKCE S256</label>
        <div class="field"><label>回调 URL</label><input class="mono" value="${escapeHtml(provider.callback_url)}" readonly></div>
        <button class="button" type="submit">保存 ${escapeHtml(provider.name)}</button><p class="form-error" data-role="error"></p>
      </form>
    </section>`;
  }

  function attachSettings(page) {
    page.querySelectorAll(".oauth-provider-form").forEach(form => form.addEventListener("submit", saveSettings));
  }

  async function saveSettings(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const values = Object.fromEntries(new FormData(form));
    const provider = form.dataset.provider;
    values.enabled = form.querySelector('[data-role="enabled"]').checked;
    values.use_pkce = form.querySelector('[data-role="pkce"]').checked;
    values.validate_id_token = Boolean(form.querySelector('[data-role="validate-id-token"]')?.checked);
    values.require_email_verified = Boolean(form.querySelector('[data-role="require-email-verified"]')?.checked);
    values.dingtalk_bypass_registration = Boolean(form.querySelector('[data-role="dingtalk-bypass-registration"]')?.checked);
    values.dingtalk_require_email = Boolean(form.querySelector('[data-role="dingtalk-require-email"]')?.checked);
    values.dingtalk_sync_corp_email = Boolean(form.querySelector('[data-role="dingtalk-sync-email"]')?.checked);
    values.dingtalk_sync_display_name = Boolean(form.querySelector('[data-role="dingtalk-sync-name"]')?.checked);
    values.dingtalk_sync_dept = Boolean(form.querySelector('[data-role="dingtalk-sync-dept"]')?.checked);
    values.clock_skew_seconds = Number(values.clock_skew_seconds || 120);
    values.clear_client_secret = Boolean(form.querySelector('[data-role="clear-secret"]')?.checked);
    const button = form.querySelector("button[type=submit]");
    const error = form.querySelector('[data-role="error"]');
    button.disabled = true; error.textContent = "";
    try {
      await api(`/api/admin/auth-providers/${encodeURIComponent(provider)}`, { method: "PUT", body: JSON.stringify(values) });
      const publicSettings = await api("/api/public/settings");
      state.oauthProviders = publicSettings.data.oauth_providers || [];
      toast("登录提供商设置已保存"); await renderRoute();
    } catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
  }

  async function renderPendingOAuth(token) {
    let pending;
    try {
      const result = await api("/api/auth/oauth/pending/inspect", { method: "POST", body: JSON.stringify({ token }) });
      pending = result.data;
    } catch (error) {
      renderAuthScreen("授权信息已失效", error.message, `<p class="auth-notice mono">${escapeHtml(error.code || "OAUTH_PENDING_INVALID")}</p>`, '<a class="text-link" href="#/overview">返回登录</a>');
      return;
    }
    renderPendingChoice(token, pending);
  }

  function pendingIdentitySummary(pending) {
    return `<div class="identity-list"><div class="identity-row"><div><strong>${escapeHtml(pending.provider_name)}</strong><span>${escapeHtml(pending.display_name || pending.email_hint || "身份验证已完成")}</span></div><span class="status">已验证</span></div></div>`;
  }

  function renderPendingChoice(token, pending) {
    const registerButton = pending.registration_enabled ? '<button class="button secondary" id="oauth-register-account" type="button">创建新账户</button>' : "";
    renderAuthScreen("完成第三方登录", `使用 ${pending.provider_name} 身份继续`, `${pendingIdentitySummary(pending)}<div class="inline-field"><button class="button" id="oauth-bind-account" type="button">绑定现有账户</button>${registerButton}</div><p class="field-hint">第三方身份不会按邮箱自动合并，绑定前需要验证本地账户密码。</p>`, '<a class="text-link" href="#/overview">取消并返回登录</a>');
    document.querySelector("#oauth-bind-account").addEventListener("click", () => renderPendingBind(token, pending));
    document.querySelector("#oauth-register-account")?.addEventListener("click", () => renderPendingRegister(token, pending));
  }

  function renderPendingBind(token, pending) {
    renderAuthScreen("绑定现有账户", `验证账户后绑定 ${pending.provider_name}`, `
      ${pendingIdentitySummary(pending)}
      <form id="oauth-pending-bind-form">
        <div class="field"><label for="oauth-bind-identifier">用户名或邮箱</label><input id="oauth-bind-identifier" name="identifier" autocomplete="username" required autofocus></div>
        <div class="field"><label for="oauth-bind-password">密码</label><input id="oauth-bind-password" name="password" type="password" maxlength="128" autocomplete="current-password" required></div>
        <div class="field"><label for="oauth-bind-totp">动态码或恢复码（启用双因素时填写）</label><input id="oauth-bind-totp" name="totp_code" autocomplete="one-time-code"></div>
        <button class="button auth-submit" type="submit">验证并绑定</button><p class="form-error" id="oauth-bind-error"></p>
      </form>`, '<a class="text-link" id="oauth-bind-back" href="#">返回选择</a><a class="text-link" href="#/overview">取消</a>');
    document.querySelector("#oauth-bind-back").addEventListener("click", event => { event.preventDefault(); renderPendingChoice(token, pending); });
    document.querySelector("#oauth-pending-bind-form").addEventListener("submit", async event => {
      event.preventDefault();
      const form = event.currentTarget;
      const button = form.querySelector("button[type=submit]");
      const error = form.querySelector("#oauth-bind-error");
      button.disabled = true; error.textContent = "";
      try {
        const values = Object.fromEntries(new FormData(form));
        values.token = token;
        if (!values.totp_code) delete values.totp_code;
        const result = await api("/api/auth/oauth/pending/bind", { method: "POST", body: JSON.stringify(values) });
        applyIdentity(result.data); location.hash = "#/overview"; renderShell(); await renderRoute(); toast("第三方身份已绑定");
      } catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
    });
  }

  function renderPendingRegister(token, pending) {
    const suggestedEmail = pending.suggested_email || "";
    renderAuthScreen("创建新账户", `使用 ${pending.provider_name} 身份注册`, `
      ${pendingIdentitySummary(pending)}
      <form id="oauth-pending-register-form">
        <div class="field"><label for="oauth-register-email">邮箱</label><input id="oauth-register-email" name="email" type="email" value="${escapeHtml(suggestedEmail)}" maxlength="254" autocomplete="email" required autofocus></div>
        <div class="field"><label for="oauth-register-password">密码</label><input id="oauth-register-password" name="password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required><span class="field-hint">8 至 128 个字符</span></div>
        <div class="field"><label for="oauth-register-confirm">确认密码</label><input id="oauth-register-confirm" name="confirm_password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required></div>
        <div id="oauth-register-verification" hidden><div class="field"><label for="oauth-register-code">邮箱验证码</label><input id="oauth-register-code" name="verify_code" maxlength="16" autocomplete="one-time-code"><span class="field-hint" id="oauth-register-code-hint">修改或未验证的邮箱需要验证码</span></div><button class="button secondary small" id="oauth-send-code" type="button">发送验证码</button></div>
        <button class="button auth-submit" type="submit">创建账户</button><p class="form-error" id="oauth-register-error"></p>
      </form>`, '<a class="text-link" id="oauth-register-back" href="#">返回选择</a><a class="text-link" href="#/overview">取消</a>');
    const form = document.querySelector("#oauth-pending-register-form");
    const emailInput = form.elements.email;
    const verification = form.querySelector("#oauth-register-verification");
    const codeInput = form.elements.verify_code;
    const requiresVerification = () => pending.email_verification_enabled && !(pending.provider_email_verified && emailInput.value.trim().toLowerCase() === suggestedEmail.trim().toLowerCase());
    const updateVerification = () => { const required = requiresVerification(); verification.hidden = !required; codeInput.required = required; };
    updateVerification();
    emailInput.addEventListener("input", updateVerification);
    document.querySelector("#oauth-register-back").addEventListener("click", event => { event.preventDefault(); renderPendingChoice(token, pending); });
    form.querySelector("#oauth-send-code").addEventListener("click", async event => {
      if (!emailInput.reportValidity()) return;
      const button = event.currentTarget;
      const hint = form.querySelector("#oauth-register-code-hint");
      button.disabled = true; hint.textContent = "正在发送...";
      try {
        const sent = await api("/api/auth/send-verification-code", { method: "POST", body: JSON.stringify({ email: emailInput.value }) });
        let remaining = Number(sent.data.countdown || 60);
        const tick = () => {
          if (!button.isConnected) return;
          button.disabled = remaining > 0;
          hint.textContent = remaining > 0 ? `验证码已发送，${remaining} 秒后可重新发送` : "没有收到邮件时可以重新发送";
          if (remaining-- > 0) setTimeout(tick, 1000);
        };
        tick();
      } catch (requestError) { hint.textContent = requestError.message; button.disabled = false; }
    });
    form.addEventListener("submit", async event => {
      event.preventDefault();
      const button = form.querySelector("button[type=submit]");
      const error = form.querySelector("#oauth-register-error");
      const values = Object.fromEntries(new FormData(form));
      error.textContent = "";
      if (values.password !== values.confirm_password) { error.textContent = "两次输入的密码不一致"; return; }
      delete values.confirm_password; values.token = token;
      if (!requiresVerification()) delete values.verify_code;
      button.disabled = true;
      try {
        const result = await api("/api/auth/oauth/pending/register", { method: "POST", body: JSON.stringify(values) });
        applyIdentity(result.data); location.hash = "#/overview"; renderShell(); await renderRoute(); toast("账户已创建并绑定第三方身份");
      } catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
    });
  }

  return { profileRows, attachProfile, settingsPanel, attachSettings, renderPendingOAuth };
})();
