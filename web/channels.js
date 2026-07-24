"use strict";

(() => {
  let channels = [];
  let groups = [];
  let accounts = [];

  async function renderAdmin(page) {
    const [channelResult, groupResult, accountResult] = await Promise.all([
      api("/api/admin/channels"),
      api("/api/admin/groups"),
      api("/api/admin/accounts"),
    ]);
    channels = channelResult.data;
    groups = groupResult.data;
    accounts = accountResult.data;
    page.innerHTML = `${pageHeader("频道定价", `${channels.length} 个频道`, '<button class="button" id="add-channel-advanced">创建频道</button>')}${channels.length ? table(channels) : emptyState("暂无频道", "创建频道并绑定路由分组", "创建频道", "empty-add-channel-advanced")}`;
    page.querySelector("#add-channel-advanced")?.addEventListener("click", () => openEditor());
    page.querySelector("#empty-add-channel-advanced")?.addEventListener("click", () => openEditor());
    page.querySelectorAll("[data-channel-advanced-action]").forEach(button => button.addEventListener("click", handleAction));
  }

  function table(items) {
    return `<div class="table-wrap"><table><thead><tr><th>频道</th><th>状态</th><th>计费模型</th><th>分组</th><th>映射</th><th>价格规则</th><th>账号统计</th><th>限制</th><th></th></tr></thead><tbody>${items.map(channel => `<tr><td><span class="cell-main">${escapeHtml(channel.name)}</span><span class="cell-sub">${escapeHtml(channel.description || "")}</span></td><td>${channel.status === "active" ? status("启用") : status("停用", "off")}</td><td>${billingSourceLabel(channel.billing_model_source)}</td><td>${channel.group_ids.length}</td><td>${mappingCount(channel.model_mapping)} 条</td><td>${channel.model_pricing.length}</td><td>${channel.account_stats_pricing_rules?.length ? `${channel.account_stats_pricing_rules.length} 条` : channel.apply_pricing_to_account_stats ? "复用频道价格" : '<span class="cell-sub">默认</span>'}</td><td>${channel.restrict_models ? status("仅定价模型") : '<span class="cell-sub">不限制</span>'}</td><td><div class="cell-actions"><button class="button quiet small" data-channel-advanced-action="edit" data-id="${channel.id}">编辑</button><button class="button quiet small" data-channel-advanced-action="toggle" data-id="${channel.id}">${channel.status === "active" ? "停用" : "启用"}</button><button class="button quiet small" data-channel-advanced-action="delete" data-id="${channel.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function billingSourceLabel(source) {
    return source === "requested" ? "请求模型" : source === "upstream" ? "上游模型" : "映射后模型";
  }

  function mappingCount(mapping) {
    return Object.values(mapping || {}).reduce((count, rules) => count + Object.keys(rules || {}).length, 0);
  }

  function openEditor(channel = null) {
    const selectedGroups = new Set(channel?.group_ids || []);
    openModal(channel ? "编辑频道" : "创建频道", `<form id="advanced-channel-form">
      <div class="field"><label for="advanced-channel-name">名称</label><input id="advanced-channel-name" name="name" value="${escapeHtml(channel?.name || "")}" maxlength="100" required autofocus></div>
      <div class="field"><label for="advanced-channel-description">说明</label><textarea id="advanced-channel-description" name="description" class="compact-textarea">${escapeHtml(channel?.description || "")}</textarea></div>
      <div class="form-grid"><div class="field"><label for="advanced-channel-status">状态</label><select id="advanced-channel-status" name="status"><option value="active" ${channel?.status !== "inactive" ? "selected" : ""}>启用</option><option value="inactive" ${channel?.status === "inactive" ? "selected" : ""}>停用</option></select></div><div class="field"><label for="billing-model-source">计费模型来源</label><select id="billing-model-source" name="billing_model_source"><option value="channel_mapped" ${channel?.billing_model_source !== "requested" && channel?.billing_model_source !== "upstream" ? "selected" : ""}>映射后模型</option><option value="requested" ${channel?.billing_model_source === "requested" ? "selected" : ""}>客户端请求模型</option><option value="upstream" ${channel?.billing_model_source === "upstream" ? "selected" : ""}>上游模型</option></select></div></div>
      <label class="switch-row"><span><strong>限制模型</strong><small>只允许价格规则中的请求模型或映射目标</small></span><input name="restrict_models" type="checkbox" ${channel?.restrict_models ? "checked" : ""}></label>
      <div class="field"><label>路由分组</label><div class="choice-grid">${groups.map(group => `<label><input type="checkbox" name="group_ids" value="${group.id}" ${selectedGroups.has(group.id) ? "checked" : ""}><span>${escapeHtml(group.name)}</span><small>${escapeHtml(group.platform_label || group.platform)} · ${group.account_ids.length} 个账号</small></label>`).join("") || '<span class="field-hint">暂无路由分组</span>'}</div></div>
      <div class="section-title compact"><h2>模型映射</h2><button class="button secondary small" type="button" id="add-model-mapping">添加映射</button></div><div id="model-mapping-rules"></div>
      <div class="section-title compact"><h2>模型价格</h2><button class="button secondary small" type="button" id="add-advanced-pricing">添加规则</button></div><div id="advanced-pricing-rules"></div>
      <label class="switch-row"><span><strong>频道价格用于账号统计</strong><small>无专用规则命中时，以倍率前的频道价格估算上游账号成本</small></span><input name="apply_pricing_to_account_stats" type="checkbox" ${channel?.apply_pricing_to_account_stats ? "checked" : ""}></label>
      <div class="section-title compact"><div><h2>账号统计价格</h2><p>按顺序匹配账号或频道内分组，不影响用户扣费</p></div><button class="button secondary small" type="button" id="add-account-stats-rule">添加规则</button></div><div id="account-stats-rules"></div>
      <p class="form-error" id="advanced-channel-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-advanced-channel">保存</button>');
    Object.entries(channel?.model_mapping || {}).forEach(([platform, rules]) => Object.entries(rules || {}).forEach(([source, target]) => addMappingRow({ platform, source, target })));
    (channel?.model_pricing?.length ? channel.model_pricing : [{ platform: "openai", models: [], billing_mode: "tokens" }]).forEach(addPricingRule);
    (channel?.account_stats_pricing_rules || []).forEach(addAccountStatsRule);
    modal.querySelector("#add-model-mapping").addEventListener("click", () => addMappingRow());
    modal.querySelector("#add-advanced-pricing").addEventListener("click", () => addPricingRule());
    modal.querySelector("#add-account-stats-rule").addEventListener("click", () => addAccountStatsRule());
    modal.querySelector("#save-advanced-channel").addEventListener("click", () => save(channel?.id));
  }

  function platformOptions(selected = "openai") {
    const values = [...new Set(["openai", "anthropic", "gemini", "grok", ...groups.map(group => group.platform).filter(Boolean)])];
    return values.map(value => `<option value="${escapeHtml(value)}" ${value === selected ? "selected" : ""}>${escapeHtml(value.toUpperCase())}</option>`).join("");
  }

  function addMappingRow(rule = {}) {
    const row = document.createElement("div");
    row.className = "pricing-interval model-mapping-row";
    row.innerHTML = `<div class="field"><label>平台</label><select name="mapping_platform">${platformOptions(rule.platform)}</select></div><div class="field"><label>请求模型</label><input name="mapping_source" value="${escapeHtml(rule.source || "")}" maxlength="128" placeholder="gpt-5-*" required></div><div class="field"><label>上游目标</label><input name="mapping_target" value="${escapeHtml(rule.target || "")}" maxlength="128" placeholder="gpt-5.2" required></div><button class="button quiet small" type="button" data-remove-mapping>移除</button>`;
    row.querySelector("[data-remove-mapping]").addEventListener("click", () => row.remove());
    modal.querySelector("#model-mapping-rules").append(row);
  }

  function addPricingRule(rule = {}, container = modal.querySelector("#advanced-pricing-rules"), ruleClass = "advanced-pricing-rule", intervalClass = "advanced-pricing-interval") {
    const row = document.createElement("div");
    row.className = `pricing-rule ${ruleClass}`;
    row.innerHTML = `<div class="form-grid"><div class="field"><label>平台</label><select name="platform">${platformOptions(rule.platform)}</select></div><div class="field"><label>计费模式</label><select name="billing_mode"><option value="tokens" ${rule.billing_mode !== "request" ? "selected" : ""}>按 Token</option><option value="request" ${rule.billing_mode === "request" ? "selected" : ""}>按请求</option></select></div></div>
      <div class="field"><label>模型或后缀通配符</label><textarea name="models" class="compact-textarea" placeholder="每行一个；例如 gpt-5-*" required>${escapeHtml((rule.models || []).join("\n"))}</textarea></div>
      <div class="pricing-grid"><div class="field token-price"><label>文本输入 $ / 1M</label><input name="input_price" type="number" min="0" step="0.000001" value="${Number(rule.input_price || 0)}"></div><div class="field token-price"><label>缓存读取 $ / 1M</label><input name="cache_read_price" type="number" min="0" step="0.000001" value="${optionalValue(rule.cache_read_price)}" placeholder="回退输入价"></div><div class="field token-price"><label>缓存写入 $ / 1M</label><input name="cache_write_price" type="number" min="0" step="0.000001" value="${optionalValue(rule.cache_write_price)}" placeholder="回退输入价"></div><div class="field token-price"><label>图片输入 $ / 1M</label><input name="image_input_price" type="number" min="0" step="0.000001" value="${optionalValue(rule.image_input_price)}" placeholder="回退文本输入"></div><div class="field token-price"><label>文本输出 $ / 1M</label><input name="output_price" type="number" min="0" step="0.000001" value="${Number(rule.output_price || 0)}"></div><div class="field token-price"><label>图片输出 $ / 1M</label><input name="image_output_price" type="number" min="0" step="0.000001" value="${optionalValue(rule.image_output_price)}" placeholder="回退文本输出"></div><div class="field request-price"><label>单次 $</label><input name="per_request_price" type="number" min="0" step="0.000001" value="${Number(rule.per_request_price || 0)}"></div><button class="button quiet small" type="button" data-remove-advanced-pricing>移除</button></div>
      <div class="token-tier-editor"><div class="section-title compact"><h3>Token 区间</h3><button class="button quiet small" type="button" data-add-advanced-interval>添加区间</button></div><div class="advanced-pricing-intervals"></div></div>`;
    (rule.intervals || []).forEach(interval => addInterval(row.querySelector(".advanced-pricing-intervals"), interval, intervalClass));
    row.querySelector("[data-add-advanced-interval]").addEventListener("click", () => addInterval(row.querySelector(".advanced-pricing-intervals"), {}, intervalClass));
    row.querySelector("[data-remove-advanced-pricing]").addEventListener("click", () => row.remove());
    const sync = () => { const tokenMode = row.querySelector('[name="billing_mode"]').value === "tokens"; row.querySelectorAll(".token-price,.token-tier-editor").forEach(element => { element.hidden = !tokenMode; }); row.querySelectorAll(".request-price").forEach(element => { element.hidden = tokenMode; }); };
    row.querySelector('[name="billing_mode"]').addEventListener("change", sync);
    container.append(row); sync();
  }

  function addInterval(container, interval = {}, intervalClass = "advanced-pricing-interval") {
    const row = document.createElement("div");
    row.className = `pricing-interval ${intervalClass}`;
    row.innerHTML = `<div class="field"><label>最小 Token（不含）</label><input name="interval_min" type="number" min="0" value="${Number(interval.min_tokens || 0)}" required></div><div class="field"><label>最大 Token（含）</label><input name="interval_max" type="number" min="1" value="${interval.max_tokens == null ? "" : Number(interval.max_tokens)}" placeholder="不限"></div><div class="field"><label>输入 $ / 1M</label><input name="interval_input" type="number" min="0" step="0.000001" value="${optionalValue(interval.input_price)}"></div><div class="field"><label>缓存读 $ / 1M</label><input name="interval_cache_read" type="number" min="0" step="0.000001" value="${optionalValue(interval.cache_read_price)}"></div><div class="field"><label>缓存写 $ / 1M</label><input name="interval_cache_write" type="number" min="0" step="0.000001" value="${optionalValue(interval.cache_write_price)}"></div><div class="field"><label>输出 $ / 1M</label><input name="interval_output" type="number" min="0" step="0.000001" value="${optionalValue(interval.output_price)}"></div><button class="button quiet small" type="button" data-remove-advanced-interval>移除</button>`;
    row.querySelector("[data-remove-advanced-interval]").addEventListener("click", () => row.remove());
    container.append(row);
  }

  function addAccountStatsRule(rule = {}) {
    const selectedGroups = new Set(rule.group_ids || []);
    const selectedAccounts = new Set(rule.account_ids || []);
    const section = document.createElement("section");
    section.className = "account-stats-rule";
    section.innerHTML = `<div class="section-title compact"><div><h3>${escapeHtml(rule.name || "新统计规则")}</h3><p>账号或分组任一命中即适用</p></div><button class="button quiet small" type="button" data-remove-account-stats-rule>移除规则</button></div>
      <div class="field"><label>规则名称</label><input name="stats_rule_name" value="${escapeHtml(rule.name || "")}" maxlength="100" required placeholder="例如 OAuth 实际成本"></div>
      <div class="field"><label>频道内分组</label><div class="choice-grid">${groups.map(group => `<label><input type="checkbox" name="stats_group_id" value="${group.id}" ${selectedGroups.has(group.id) ? "checked" : ""}><span>${escapeHtml(group.name)}</span><small>${escapeHtml(group.platform_label || group.platform)}</small></label>`).join("") || '<span class="field-hint">暂无分组</span>'}</div></div>
      <div class="field"><label>指定上游账号</label><div class="choice-grid account-stats-account-grid">${accounts.map(account => `<label><input type="checkbox" name="stats_account_id" value="${account.id}" ${selectedAccounts.has(account.id) ? "checked" : ""}><span>${escapeHtml(account.name)}</span><small>#${account.id} · ${account.kind === "oauth" ? "OAuth" : "API Key"}</small></label>`).join("") || '<span class="field-hint">暂无上游账号</span>'}</div><span class="field-hint">至少选择一个分组或账号；上方选择的分组才可作为规则范围</span></div>
      <div class="section-title compact"><h3>专用模型价格</h3><button class="button quiet small" type="button" data-add-account-stats-pricing>添加价格</button></div><div class="account-stats-pricing"></div>`;
    const pricing = section.querySelector(".account-stats-pricing");
    (rule.pricing?.length ? rule.pricing : [{ platform: "openai", models: [], billing_mode: "tokens" }]).forEach(item => addPricingRule(item, pricing, "stats-pricing-rule", "stats-pricing-interval"));
    section.querySelector("[data-add-account-stats-pricing]").addEventListener("click", () => addPricingRule({}, pricing, "stats-pricing-rule", "stats-pricing-interval"));
    section.querySelector("[data-remove-account-stats-rule]").addEventListener("click", () => section.remove());
    section.querySelector('[name="stats_rule_name"]').addEventListener("input", event => { section.querySelector("h3").textContent = event.currentTarget.value.trim() || "新统计规则"; });
    modal.querySelector("#account-stats-rules").append(section);
  }

  function collectPricing(scope, ruleSelector, intervalSelector) {
    return [...scope.querySelectorAll(ruleSelector)].map(row => ({
      platform: row.querySelector('[name="platform"]').value,
      billing_mode: row.querySelector('[name="billing_mode"]').value,
      models: parseModelList(row.querySelector('[name="models"]').value),
      input_price: Number(row.querySelector('[name="input_price"]').value),
      output_price: Number(row.querySelector('[name="output_price"]').value),
      cache_read_price: optionalNumber(row.querySelector('[name="cache_read_price"]')),
      cache_write_price: optionalNumber(row.querySelector('[name="cache_write_price"]')),
      image_input_price: optionalNumber(row.querySelector('[name="image_input_price"]')),
      image_output_price: optionalNumber(row.querySelector('[name="image_output_price"]')),
      per_request_price: Number(row.querySelector('[name="per_request_price"]').value),
      intervals: [...row.querySelectorAll(intervalSelector)].map(interval => ({
        min_tokens: Number(interval.querySelector('[name="interval_min"]').value),
        max_tokens: optionalNumber(interval.querySelector('[name="interval_max"]')),
        input_price: optionalNumber(interval.querySelector('[name="interval_input"]')),
        output_price: optionalNumber(interval.querySelector('[name="interval_output"]')),
        cache_read_price: optionalNumber(interval.querySelector('[name="interval_cache_read"]')),
        cache_write_price: optionalNumber(interval.querySelector('[name="interval_cache_write"]')),
      })),
    }));
  }

  function optionalValue(value) { return value == null ? "" : Number(value); }
  function optionalNumber(input) { return input.value === "" ? null : Number(input.value); }

  async function save(id) {
    const form = modal.querySelector("#advanced-channel-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    values.restrict_models = form.elements.restrict_models.checked;
    values.group_ids = [...form.querySelectorAll('[name="group_ids"]:checked')].map(input => Number(input.value));
    values.model_mapping = {};
    for (const row of form.querySelectorAll(".model-mapping-row")) {
      const platform = row.querySelector('[name="mapping_platform"]').value;
      const source = row.querySelector('[name="mapping_source"]').value.trim();
      const target = row.querySelector('[name="mapping_target"]').value.trim();
      values.model_mapping[platform] ||= {};
      if (values.model_mapping[platform][source]) {
        modal.querySelector("#advanced-channel-error").textContent = `映射规则重复：${source}`;
        return;
      }
      values.model_mapping[platform][source] = target;
    }
    values.model_pricing = collectPricing(form, ".advanced-pricing-rule", ".advanced-pricing-interval");
    values.apply_pricing_to_account_stats = form.elements.apply_pricing_to_account_stats.checked;
    values.account_stats_pricing_rules = [...form.querySelectorAll(".account-stats-rule")].map(rule => ({
      name: rule.querySelector('[name="stats_rule_name"]').value.trim(),
      group_ids: [...rule.querySelectorAll('[name="stats_group_id"]:checked')].map(input => Number(input.value)),
      account_ids: [...rule.querySelectorAll('[name="stats_account_id"]:checked')].map(input => Number(input.value)),
      pricing: collectPricing(rule, ".stats-pricing-rule", ".stats-pricing-interval"),
    }));
    const channelGroups = new Set(values.group_ids);
    const invalidStatsRule = values.account_stats_pricing_rules.find(rule => (!rule.group_ids.length && !rule.account_ids.length) || rule.group_ids.some(groupId => !channelGroups.has(groupId)));
    if (invalidStatsRule) {
      modal.querySelector("#advanced-channel-error").textContent = "账号统计规则至少选择一个范围，且规则分组必须已绑定当前频道";
      return;
    }
    const button = modal.querySelector("#save-advanced-channel");
    button.disabled = true;
    try { await api(id ? `/api/admin/channels/${id}` : "/api/admin/channels", { method: id ? "PUT" : "POST", body: JSON.stringify(values) }); closeModal(); toast("频道设置已保存"); await renderRoute(); }
    catch (error) { modal.querySelector("#advanced-channel-error").textContent = error.message; button.disabled = false; }
  }

  function handleAction(event) {
    const channel = channels.find(item => String(item.id) === event.currentTarget.dataset.id);
    if (!channel) return;
    const action = event.currentTarget.dataset.channelAdvancedAction;
    if (action === "edit") return openEditor(channel);
    if (action === "toggle") return updateStatus(channel);
    openModal("删除频道", `<p>确认删除 <strong>${escapeHtml(channel.name)}</strong>？定价和映射规则会一并删除。</p><p class="form-error" id="channel-delete-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-delete-channel-advanced">删除</button>');
    modal.querySelector("#confirm-delete-channel-advanced").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try { await api(`/api/admin/channels/${channel.id}`, { method: "DELETE" }); closeModal(); toast("频道已删除"); await renderRoute(); }
      catch (error) { modal.querySelector("#channel-delete-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function updateStatus(channel) {
    try {
      await api(`/api/admin/channels/${channel.id}`, { method: "PUT", body: JSON.stringify({ ...channel, status: channel.status === "active" ? "inactive" : "active" }) });
      toast("频道状态已更新"); await renderRoute();
    } catch (error) { toast(error.message, true); }
  }

  window.Sub2MiniChannels = { renderAdmin };
})();
