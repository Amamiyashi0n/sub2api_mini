"use strict";

window.Sub2MiniAccountTools = (() => {
  function attach(page) {
    page.querySelectorAll("[data-account-tool]").forEach(button => button.addEventListener("click", handle));
  }

  async function handle(event) {
    const button = event.currentTarget;
    const account = currentAccounts.find(item => String(item.id) === button.dataset.id);
    if (!account) return;
    closeUpstreamAccountMenu();
    const action = button.dataset.accountTool;
    if (action === "reauth") return openReauth(account);
    if (action === "spark") return openSparkConfirm(account);
    if (action === "duplicate") {
      if (!confirm(`复制“${account.name}”并以停用状态创建独立账号？`)) return;
      button.disabled = true;
      try {
        const result = await api(`/api/admin/accounts/${account.id}/duplicate`, { method: "POST", body: "{}" });
        toast(`已创建 ${result.data.name}，启用前请检查凭证与分组`);
        await renderRoute();
      } catch (error) { toast(error.message, true); button.disabled = false; }
      return;
    }
    button.disabled = true;
    try { await openStats(account, 30); button.disabled = false; }
    catch (error) { closeModal(); toast(error.message, true); button.disabled = false; }
  }

  async function openStats(account, days) {
    openModal(`${account.name} · 使用统计`, `<div class="boot-screen"><p>正在载入</p></div>`, `<button class="button" data-close-modal>关闭</button>`);
    const result = await api(`/api/admin/accounts/${account.id}/stats?days=${days}`);
    const data = result.data;
    const summary = data.summary;
    const body = modal.querySelector(".modal-body");
    body.innerHTML = `<div class="account-stats-toolbar"><span>UTC ${escapeHtml(summary.utc_offset)}</span><div class="segmented">${[1, 7, 30, 90].map(value => `<button class="button ${days === value ? "" : "quiet"} small" data-account-stats-days="${value}">${value === 1 ? "今天" : `${value} 天`}</button>`).join("")}</div></div>
      <section class="metric-grid account-stat-metrics">
        ${metric("请求", formatNumber(summary.total_requests))}
        ${metric("成功率", `${Number(summary.success_rate).toFixed(1)}%`, summary.failed_requests ? "warn" : "good")}
        ${metric("总 Token", formatNumber(summary.total_tokens))}
        ${metric("缓存读 / 写", `${formatNumber(summary.cached_input_tokens)} / ${formatNumber(summary.cache_write_tokens)}`)}
        ${metric("上游账号成本", formatUsdMicros(summary.total_cost_microusd))}
        ${metric("用户计费", formatUsdMicros(summary.total_user_cost_microusd))}
        ${metric("平均耗时", `${formatNumber(Math.round(summary.avg_duration_ms))} ms`)}
      </section>
      <div class="usage-analytics-grid account-stat-distributions">
        <section><div class="section-title"><h2>模型分布</h2></div>${bars(data.models, "model")}</section>
        <section><div class="section-title"><h2>端点分布</h2></div>${bars(data.endpoints, "endpoint")}</section>
      </div>
      <section><div class="section-title"><div><h2>每日趋势</h2><p>活跃 ${summary.actual_days_used} 天 · 日均 ${formatNumber(Math.round(summary.avg_daily_requests))} 次请求</p></div></div>${historyTable(data.history)}</section>`;
    body.querySelectorAll("[data-account-stats-days]").forEach(button => button.addEventListener("click", async click => {
      click.currentTarget.disabled = true;
      try { await openStats(account, Number(click.currentTarget.dataset.accountStatsDays)); }
      catch (error) { toast(error.message, true); click.currentTarget.disabled = false; }
    }));
  }

  function bars(rows, labelKey) {
    if (!rows.length) return emptyState("暂无分布数据", "所选范围内没有网关请求");
    const max = Math.max(...rows.map(row => Number(row.requests) || 0), 1);
    return `<div class="usage-bars">${rows.map(row => `<div><span title="${escapeHtml(row[labelKey])}">${escapeHtml(row[labelKey])}</span><div><i style="width:${Math.max(2, Number(row.requests) / max * 100)}%"></i></div><strong>${formatNumber(row.requests)}</strong></div>`).join("")}</div>`;
  }

  function historyTable(rows) {
    if (!rows.length) return emptyState("暂无使用记录", "所选时间范围内没有网关请求");
    return `<div class="table-wrap"><table><thead><tr><th>日期</th><th>请求</th><th>成功 / 失败</th><th>Token</th><th>账号成本</th><th>用户计费</th></tr></thead><tbody>${[...rows].reverse().map(row => `<tr><td>${escapeHtml(row.date)}</td><td>${formatNumber(row.requests)}</td><td>${formatNumber(row.successful_requests)} / ${formatNumber(row.failed_requests)}</td><td>${formatNumber(row.tokens)}</td><td>${formatUsdMicros(row.cost_microusd)}</td><td>${formatUsdMicros(row.user_cost_microusd)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function openReauth(account) {
    openModal(`${account.name} · OAuth 重认证`, `<form id="account-reauth-form"><div class="sensitive-notice"><strong>替换 OAuth 凭证</strong><span>新凭证会立即加密保存，账号优先级、并发、代理和分组保持不变。</span></div><div class="field"><label for="account-reauth-content">Codex auth.json</label><textarea id="account-reauth-content" name="content" spellcheck="false" required autofocus></textarea></div><p class="form-error" id="account-reauth-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button secondary" id="start-account-browser-reauth">浏览器 OAuth</button><button class="button" id="save-account-reauth">导入 auth.json</button>`);
    modal.querySelector("#start-account-browser-reauth").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try {
        const result = await api("/api/admin/oauth/start", { method: "POST", body: JSON.stringify({ account_id: account.id }) });
        window.open(result.data.auth_url, "_blank", "noopener");
        closeModal(); toast("已打开 OAuth 重认证窗口");
      } catch (error) { modal.querySelector("#account-reauth-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
    modal.querySelector("#save-account-reauth").addEventListener("click", async event => {
      const form = modal.querySelector("#account-reauth-form");
      if (!form.reportValidity()) return;
      event.currentTarget.disabled = true;
      try {
        await api(`/api/admin/accounts/${account.id}/reauth`, { method: "POST", body: JSON.stringify({ content: form.elements.content.value }) });
        closeModal(); toast("OAuth 凭证已更新"); await renderRoute();
      } catch (error) { modal.querySelector("#account-reauth-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  function openSparkConfirm(account) {
    openModal("创建 Spark 影子", `<div class="sensitive-notice"><strong>${escapeHtml(account.name)}</strong><span>影子共享母账号 OAuth 凭证与代理，拥有独立调度、分组、统计和定时测试，并以停用状态创建。</span></div><p class="form-error" id="spark-shadow-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="create-spark-shadow">创建</button>`);
    modal.querySelector("#create-spark-shadow").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try {
        const result = await api(`/api/admin/accounts/${account.id}/spark-shadow`, { method: "POST", body: "{}" });
        closeModal(); toast(`已创建 ${result.data.name}，检查调度配置后再启用`); await renderRoute();
      } catch (error) { modal.querySelector("#spark-shadow-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function openErrorRules() {
    closeAccountToolsMenu();
    openModal("错误透传规则", `<div class="boot-screen"><p>正在载入</p></div>`, `<button class="button secondary" data-close-modal>关闭</button><button class="button" id="add-error-rule">添加规则</button>`);
    try {
      const result = await api("/api/admin/error-passthrough-rules");
      renderErrorRules(result.data);
    } catch (error) { modal.querySelector(".modal-body").innerHTML = emptyState("载入失败", error.message); }
  }

  function renderErrorRules(rules) {
    const body = modal.querySelector(".modal-body");
    body.innerHTML = rules.length ? `<div class="table-wrap"><table><thead><tr><th>规则</th><th>匹配</th><th>响应</th><th>状态</th><th>操作</th></tr></thead><tbody>${rules.map(rule => `<tr><td><span class="cell-main">${escapeHtml(rule.name)}</span><span class="cell-sub">优先级 ${rule.priority}</span></td><td><span class="cell-main">${rule.error_codes.length ? rule.error_codes.join(", ") : "任意状态码"}</span><span class="cell-sub">${escapeHtml(rule.keywords.join(", ") || "无关键词")} · ${rule.match_mode === "all" ? "全部匹配" : "任一匹配"}</span></td><td><span class="cell-main">${rule.passthrough_code ? "透传状态码" : `改为 ${rule.response_code}`}</span><span class="cell-sub">${rule.passthrough_body ? "透传响应体" : escapeHtml(rule.custom_message || "自定义消息")}</span></td><td>${rule.enabled ? status("启用") : status("停用", "off")}</td><td><div class="cell-actions"><button class="button quiet small" data-error-rule-edit="${rule.id}">编辑</button><button class="button quiet small" data-error-rule-delete="${rule.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>` : emptyState("暂无错误透传规则", "添加后将按优先级匹配上游错误响应");
    modal.querySelector("#add-error-rule").onclick = () => openErrorRuleForm();
    body.querySelectorAll("[data-error-rule-edit]").forEach(button => button.addEventListener("click", () => openErrorRuleForm(rules.find(rule => String(rule.id) === button.dataset.errorRuleEdit))));
    body.querySelectorAll("[data-error-rule-delete]").forEach(button => button.addEventListener("click", async () => {
      const rule = rules.find(item => String(item.id) === button.dataset.errorRuleDelete);
      if (!confirm(`确认删除规则“${rule?.name || button.dataset.errorRuleDelete}”？`)) return;
      button.disabled = true;
      try { await api(`/api/admin/error-passthrough-rules/${button.dataset.errorRuleDelete}`, { method: "DELETE" }); toast("错误透传规则已删除"); await openErrorRules(); }
      catch (error) { toast(error.message, true); button.disabled = false; }
    }));
  }

  function openErrorRuleForm(rule = null) {
    openModal(rule ? "编辑错误透传规则" : "添加错误透传规则", `<form id="error-rule-form">
      <div class="form-grid"><div class="field"><label for="error-rule-name">名称</label><input id="error-rule-name" name="name" maxlength="100" value="${escapeHtml(rule?.name || "")}" required autofocus></div><div class="field"><label for="error-rule-priority">优先级</label><input id="error-rule-priority" name="priority" type="number" value="${rule?.priority ?? 0}" required></div></div>
      <div class="form-grid"><div class="field"><label for="error-rule-codes">HTTP 状态码</label><input id="error-rule-codes" name="error_codes" value="${escapeHtml((rule?.error_codes || []).join(", "))}" placeholder="400, 429, 502"></div><div class="field"><label for="error-rule-mode">匹配方式</label><select id="error-rule-mode" name="match_mode"><option value="any" ${rule?.match_mode !== "all" ? "selected" : ""}>任一条件</option><option value="all" ${rule?.match_mode === "all" ? "selected" : ""}>全部条件</option></select></div></div>
      <div class="field"><label for="error-rule-keywords">响应关键词</label><input id="error-rule-keywords" name="keywords" value="${escapeHtml((rule?.keywords || []).join(", "))}" placeholder="context limit, rate limit"><span class="field-hint">状态码和关键词至少填写一项，关键词不区分大小写</span></div>
      <div class="form-grid"><label class="switch-row"><span><strong>透传状态码</strong><small>关闭后使用右侧状态码</small></span><input name="passthrough_code" type="checkbox" ${rule?.passthrough_code === false ? "" : "checked"}></label><div class="field"><label for="error-rule-response-code">替换状态码</label><input id="error-rule-response-code" name="response_code" type="number" min="100" max="599" value="${rule?.response_code || 502}"></div></div>
      <label class="switch-row"><span><strong>透传响应体</strong><small>关闭后返回自定义错误消息</small></span><input name="passthrough_body" type="checkbox" ${rule?.passthrough_body === false ? "" : "checked"}></label>
      <div class="field"><label for="error-rule-message">自定义消息</label><input id="error-rule-message" name="custom_message" value="${escapeHtml(rule?.custom_message || "")}" placeholder="上游请求失败"></div>
      <div class="form-grid"><label class="switch-row"><span><strong>启用规则</strong></span><input name="enabled" type="checkbox" ${rule?.enabled === false ? "" : "checked"}></label><label class="switch-row"><span><strong>跳过错误监控</strong></span><input name="skip_monitoring" type="checkbox" ${rule?.skip_monitoring ? "checked" : ""}></label></div>
      <div class="field"><label for="error-rule-description">说明</label><textarea class="compact-textarea" id="error-rule-description" name="description">${escapeHtml(rule?.description || "")}</textarea></div>
      <p class="form-error" id="error-rule-error"></p></form>`, `<button class="button secondary" id="back-error-rules">返回</button><button class="button" id="save-error-rule">保存</button>`);
    modal.querySelector("#back-error-rules").addEventListener("click", openErrorRules);
    modal.querySelector("#save-error-rule").addEventListener("click", async event => {
      const form = modal.querySelector("#error-rule-form");
      if (!form.reportValidity()) return;
      const values = Object.fromEntries(new FormData(form));
      const parseNumbers = value => String(value || "").split(/[\s,]+/).filter(Boolean).map(Number);
      const parseWords = value => String(value || "").split(",").map(item => item.trim()).filter(Boolean);
      const payload = { name: values.name, priority: Number(values.priority), error_codes: parseNumbers(values.error_codes), keywords: parseWords(values.keywords), match_mode: values.match_mode, platforms: ["openai"], passthrough_code: form.elements.passthrough_code.checked, response_code: Number(values.response_code), passthrough_body: form.elements.passthrough_body.checked, custom_message: values.custom_message || null, enabled: form.elements.enabled.checked, skip_monitoring: form.elements.skip_monitoring.checked, description: values.description || null };
      event.currentTarget.disabled = true;
      try { await api(rule ? `/api/admin/error-passthrough-rules/${rule.id}` : "/api/admin/error-passthrough-rules", { method: rule ? "PUT" : "POST", body: JSON.stringify(payload) }); toast(rule ? "错误透传规则已更新" : "错误透传规则已添加"); await openErrorRules(); }
      catch (error) { modal.querySelector("#error-rule-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function openTlsProfiles() {
    closeAccountToolsMenu();
    openModal("TLS 指纹模板", `<div class="boot-screen"><p>正在载入</p></div>`, `<button class="button secondary" data-close-modal>关闭</button><button class="button" id="add-tls-profile">添加模板</button>`);
    try {
      const result = await api("/api/admin/tls-fingerprint-profiles");
      currentTlsProfiles = result.data;
      renderTlsProfiles(result.data);
    } catch (error) { modal.querySelector(".modal-body").innerHTML = emptyState("载入失败", error.message); }
  }

  function renderTlsProfiles(profiles) {
    const body = modal.querySelector(".modal-body");
    body.innerHTML = profiles.length ? `<div class="table-wrap"><table><thead><tr><th>模板</th><th>TLS 版本</th><th>ALPN</th><th>参数</th><th>操作</th></tr></thead><tbody>${profiles.map(profile => `<tr><td><span class="cell-main">${escapeHtml(profile.name)}</span><span class="cell-sub">${escapeHtml(profile.description || "-")}</span></td><td class="mono">${escapeHtml(profile.supported_versions.map(tlsValue).join(", ") || "默认")}</td><td class="mono">${escapeHtml(profile.alpn_protocols.join(", ") || "默认")}</td><td><span class="cell-sub">${profile.cipher_suites.length} 套件 · ${profile.key_share_groups.length || profile.curves.length} 组</span></td><td><div class="cell-actions"><button class="button quiet small" data-tls-edit="${profile.id}">编辑</button><button class="button quiet small" data-tls-delete="${profile.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>` : emptyState("暂无 TLS 指纹模板", "添加后可在账号新增或编辑时绑定");
    modal.querySelector("#add-tls-profile").onclick = () => openTlsProfileForm();
    body.querySelectorAll("[data-tls-edit]").forEach(button => button.addEventListener("click", () => openTlsProfileForm(profiles.find(profile => String(profile.id) === button.dataset.tlsEdit))));
    body.querySelectorAll("[data-tls-delete]").forEach(button => button.addEventListener("click", async () => {
      const profile = profiles.find(item => String(item.id) === button.dataset.tlsDelete);
      if (!confirm(`确认删除 TLS 模板“${profile?.name || button.dataset.tlsDelete}”？绑定账号将恢复默认 TLS。`)) return;
      button.disabled = true;
      try { await api(`/api/admin/tls-fingerprint-profiles/${button.dataset.tlsDelete}`, { method: "DELETE" }); toast("TLS 指纹模板已删除"); await openTlsProfiles(); }
      catch (error) { toast(error.message, true); button.disabled = false; }
    }));
  }

  function tlsValue(value) { return Number(value) === 0x0304 ? "TLS 1.3" : Number(value) === 0x0303 ? "TLS 1.2" : `0x${Number(value).toString(16)}`; }
  function arrayValue(profile, key) { return escapeHtml((profile?.[key] || []).join(", ")); }

  function openTlsProfileForm(profile = null) {
    const arrayField = (key, label, placeholder = "") => `<div class="field"><label for="tls-${key}">${label}</label><input id="tls-${key}" name="${key}" value="${arrayValue(profile, key)}" placeholder="${placeholder}"></div>`;
    openModal(profile ? "编辑 TLS 指纹模板" : "添加 TLS 指纹模板", `<form id="tls-profile-form">
      <div class="form-grid"><div class="field"><label for="tls-name">名称</label><input id="tls-name" name="name" maxlength="100" value="${escapeHtml(profile?.name || "")}" required autofocus></div><label class="switch-row"><span><strong>GREASE</strong><small>记录配置；rustls 会使用自身兼容策略</small></span><input name="enable_grease" type="checkbox" ${profile?.enable_grease ? "checked" : ""}></label></div>
      <div class="field"><label for="tls-description">说明</label><input id="tls-description" name="description" value="${escapeHtml(profile?.description || "")}"></div>
      <details class="tool-import-details"><summary>粘贴 JSON 或 YAML 参数</summary><div class="field"><textarea class="compact-textarea" id="tls-import" spellcheck="false" placeholder="cipher_suites: [4865, 4866]\nalpn_protocols: [h2, http/1.1]"></textarea></div><button class="button secondary small" id="apply-tls-import" type="button">填入表单</button></details>
      <div class="form-grid">${arrayField("cipher_suites", "Cipher Suites", "4865, 4866, 4867")}${arrayField("supported_versions", "TLS 版本", "772, 771")}${arrayField("curves", "Curves", "29, 23, 24")}${arrayField("key_share_groups", "Key Share Groups", "29, 23")}${arrayField("signature_algorithms", "Signature Algorithms")}${arrayField("point_formats", "Point Formats", "0")}${arrayField("psk_modes", "PSK Modes")}${arrayField("extensions", "Extensions")}${arrayField("alpn_protocols", "ALPN", "h2, http/1.1")}</div>
      <p class="form-error" id="tls-profile-error"></p></form>`, `<button class="button secondary" id="back-tls-profiles">返回</button><button class="button" id="save-tls-profile">保存</button>`);
    modal.querySelector("#back-tls-profiles").addEventListener("click", openTlsProfiles);
    modal.querySelector("#apply-tls-import").addEventListener("click", () => applyTlsImport(modal.querySelector("#tls-import").value));
    modal.querySelector("#save-tls-profile").addEventListener("click", async event => {
      const form = modal.querySelector("#tls-profile-form");
      if (!form.reportValidity()) return;
      const values = Object.fromEntries(new FormData(form));
      const numbers = value => String(value || "").split(/[\s,]+/).filter(Boolean).map(item => Number(item));
      const payload = { name: values.name, description: values.description || null, enable_grease: form.elements.enable_grease.checked, cipher_suites: numbers(values.cipher_suites), curves: numbers(values.curves), point_formats: numbers(values.point_formats), signature_algorithms: numbers(values.signature_algorithms), alpn_protocols: String(values.alpn_protocols || "").split(/[\s,]+/).filter(Boolean), supported_versions: numbers(values.supported_versions), key_share_groups: numbers(values.key_share_groups), psk_modes: numbers(values.psk_modes), extensions: numbers(values.extensions) };
      event.currentTarget.disabled = true;
      try { await api(profile ? `/api/admin/tls-fingerprint-profiles/${profile.id}` : "/api/admin/tls-fingerprint-profiles", { method: profile ? "PUT" : "POST", body: JSON.stringify(payload) }); toast(profile ? "TLS 指纹模板已更新" : "TLS 指纹模板已添加"); await openTlsProfiles(); }
      catch (error) { modal.querySelector("#tls-profile-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  function applyTlsImport(raw) {
    try {
      let parsed;
      try { parsed = JSON.parse(raw); }
      catch (_) {
        parsed = {};
        raw.split(/\r?\n/).forEach(line => {
          const match = line.match(/^\s*([a-z_]+)\s*:\s*\[?(.*?)\]?\s*$/i);
          if (match) parsed[match[1]] = match[2].split(",").map(item => item.trim().replace(/^['"]|['"]$/g, "")).filter(Boolean);
        });
      }
      ["cipher_suites", "curves", "point_formats", "signature_algorithms", "alpn_protocols", "supported_versions", "key_share_groups", "psk_modes", "extensions"].forEach(key => {
        if (Array.isArray(parsed[key])) modal.querySelector(`[name="${key}"]`).value = parsed[key].join(", ");
      });
      toast("TLS 参数已填入表单");
    } catch (error) { modal.querySelector("#tls-profile-error").textContent = error.message || "无法解析 TLS 参数"; }
  }

  let crsSyncState = null;

  function openCrsSync() {
    closeAccountToolsMenu();
    crsSyncState = { base_url: "", username: "", password: "", sync_proxies: true, preview: null, selected: new Set() };
    renderCrsSyncInput();
  }

  function renderCrsSyncInput() {
    const state = crsSyncState;
    openModal("从 CRS 同步账号", `<form id="crs-sync-form"><p class="crs-sync-description">从 Claude Relay Service 导入账号，并与本地已同步账号保持一致。</p><div class="crs-behavior-note">已有账号只更新 CRS 返回的字段，缺失字段继续保留；凭据按字段合并。关闭同步代理时，已有账号的代理配置保持不变。</div><div class="crs-version-note">需要 CRS v1.1.240 或更高版本。</div><div class="field"><label for="crs-base-url">CRS 地址</label><input id="crs-base-url" name="base_url" type="url" value="${escapeHtml(state.base_url)}" placeholder="http://127.0.0.1:3000" required autofocus></div><div class="form-grid"><div class="field"><label for="crs-username">用户名</label><input id="crs-username" name="username" value="${escapeHtml(state.username)}" autocomplete="username" required></div><div class="field"><label for="crs-password">密码</label><input id="crs-password" name="password" type="password" value="${escapeHtml(state.password)}" autocomplete="current-password" required></div></div><label class="crs-proxy-option"><input name="sync_proxies" type="checkbox" ${state.sync_proxies ? "checked" : ""}><span>同步代理配置</span></label><p class="form-error" id="crs-sync-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="preview-crs-sync">预览</button>`);
    modal.classList.add("crs-sync-modal");
    modal.querySelector("#preview-crs-sync").addEventListener("click", previewCrsSync);
  }

  async function previewCrsSync(event) {
    const form = modal.querySelector("#crs-sync-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    const request = { base_url: values.base_url.trim(), username: values.username.trim(), password: values.password, sync_proxies: form.elements.sync_proxies.checked };
    Object.assign(crsSyncState, request);
    event.currentTarget.disabled = true;
    try {
      const result = await api("/api/admin/accounts/sync/crs/preview", { method: "POST", body: JSON.stringify(request) });
      const data = result.data;
      const newRows = data.new_accounts || [];
      crsSyncState.preview = data;
      crsSyncState.selected = new Set(newRows.map(item => item.crs_account_id));
      renderCrsSyncPreview();
    } catch (error) { modal.querySelector("#crs-sync-error").textContent = error.message; event.currentTarget.disabled = false; }
  }

  function crsAccountTypeLabel(item) {
    const platform = item.platform === "anthropic" ? "Anthropic" : item.platform === "openai" ? "OpenAI" : item.platform;
    const type = ({ oauth: "OAuth", setup_token: "Setup Token", "setup-token": "Setup Token", api_key: "API Key", apikey: "API Key" })[item.type] || item.type;
    return `${platform} / ${type}`;
  }

  function renderCrsSyncPreview() {
    const data = crsSyncState.preview;
    const existingRows = data.existing_accounts || [];
    const newRows = data.new_accounts || [];
    const row = (item, selectable) => `<label class="crs-account-row ${selectable ? "selectable" : ""}">${selectable ? `<input type="checkbox" name="crs_account_id" value="${escapeHtml(item.crs_account_id)}" ${crsSyncState.selected.has(item.crs_account_id) ? "checked" : ""}>` : ""}<span class="crs-account-badge ${item.platform === "anthropic" ? "anthropic" : ""}">${escapeHtml(crsAccountTypeLabel(item))}</span><strong title="${escapeHtml(item.name)}">${escapeHtml(item.name)}</strong></label>`;
    openModal("从 CRS 同步账号", `<div class="crs-preview-content">${existingRows.length ? `<section class="crs-preview-group existing"><div class="crs-preview-heading"><strong>已存在的账号</strong><span>${existingRows.length}</span></div><div class="crs-account-list">${existingRows.map(item => row(item, false)).join("")}</div></section>` : ""}${newRows.length ? `<section class="crs-preview-group"><div class="crs-preview-heading"><strong>新账号</strong><span>${newRows.length}</span><div><button type="button" id="select-all-crs">全选</button><button type="button" id="select-none-crs">取消</button></div></div><div class="crs-account-list">${newRows.map(item => row(item, true)).join("")}</div><small class="crs-selected-count">已选择 ${crsSyncState.selected.size} 个账号</small></section>` : `<div class="crs-no-new">没有新账号。继续同步后将更新 ${existingRows.length} 个已有账号。</div>`}<div class="crs-preview-options"><span>同步代理配置</span><strong class="${crsSyncState.sync_proxies ? "enabled" : ""}">${crsSyncState.sync_proxies ? "是" : "否"}</strong></div>${data.unsupported_count ? `<div class="crs-unsupported-note">另有 ${data.unsupported_count} 个 Mini 不支持的平台或账号类型，已忽略。</div>` : ""}<p class="form-error" id="crs-sync-error"></p></div>`, `<button class="button secondary" id="back-crs-sync">返回</button><button class="button" id="run-crs-sync">开始同步</button>`);
    modal.classList.add("crs-sync-modal");
    const updateSelection = () => {
      crsSyncState.selected = new Set([...modal.querySelectorAll("[name=crs_account_id]:checked")].map(input => input.value));
      modal.querySelector(".crs-selected-count").textContent = `已选择 ${crsSyncState.selected.size} 个账号`;
      modal.querySelector("#run-crs-sync").disabled = newRows.length > 0 && crsSyncState.selected.size === 0;
    };
    modal.querySelectorAll("[name=crs_account_id]").forEach(input => input.addEventListener("change", updateSelection));
    modal.querySelector("#select-all-crs")?.addEventListener("click", () => { modal.querySelectorAll("[name=crs_account_id]").forEach(input => { input.checked = true; }); updateSelection(); });
    modal.querySelector("#select-none-crs")?.addEventListener("click", () => { modal.querySelectorAll("[name=crs_account_id]").forEach(input => { input.checked = false; }); updateSelection(); });
    modal.querySelector("#back-crs-sync").addEventListener("click", renderCrsSyncInput);
    modal.querySelector("#run-crs-sync").addEventListener("click", click => runCrsSync(click.currentTarget));
  }

  async function runCrsSync(button) {
    const request = { base_url: crsSyncState.base_url, username: crsSyncState.username, password: crsSyncState.password, sync_proxies: crsSyncState.sync_proxies, selected_account_ids: [...crsSyncState.selected] };
    button.disabled = true;
    try {
      const result = await api("/api/admin/accounts/sync/crs", { method: "POST", body: JSON.stringify(request) });
      const data = result.data;
      const errors = (data.items || []).filter(item => item.action === "failed" || (item.action === "skipped" && item.error !== "not selected"));
      openModal("从 CRS 同步账号", `<div class="crs-result"><strong>同步结果</strong><p>新增 ${data.created} 个，更新 ${data.updated} 个，跳过 ${data.skipped} 个，失败 ${data.failed} 个。</p>${errors.length ? `<div class="import-errors">${errors.map(item => `<div><strong>${escapeHtml(item.kind || "account")} ${escapeHtml(item.crs_account_id)}</strong><span>${escapeHtml(item.action)}${item.error ? `: ${escapeHtml(item.error)}` : ""}</span></div>`).join("")}</div>` : `<div class="crs-result-success">账号同步已完成，没有需要处理的错误。</div>`}</div>`, `<button class="button secondary" id="finish-crs-sync">关闭</button>`);
      modal.classList.add("crs-sync-modal");
      modal.querySelector("#finish-crs-sync").addEventListener("click", async () => { closeModal(); await renderRoute(); });
    } catch (error) { modal.querySelector("#crs-sync-error").textContent = error.message; button.disabled = false; }
  }

  return { attach, openCrsSync, openErrorRules, openTlsProfiles };
})();
