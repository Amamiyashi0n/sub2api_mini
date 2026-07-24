"use strict";

(() => {
  let monitors = [];
  let templates = [];

  async function render(page) {
    const [monitorResult, templateResult] = await Promise.all([
      api("/api/admin/channel-monitors"),
      api("/api/admin/channel-monitor-templates"),
    ]);
    monitors = monitorResult.data;
    templates = templateResult.data;
    page.innerHTML = `${pageHeader("频道监控", `${monitors.length} 个监控 · ${templates.length} 个模板`, '<button class="button secondary" id="add-monitor-template">创建模板</button><button class="button" id="add-monitor">创建监控</button>')}
      <section><div class="section-title"><div><h2>请求模板</h2><p>应用时复制请求头与请求体快照</p></div></div>${templateTable()}</section>
      <section><div class="section-title"><div><h2>监控配置</h2><p>业务请求与 Origin Ping 分开记录</p></div></div>${monitorTable()}</section>`;
    page.querySelector("#add-monitor").addEventListener("click", () => openMonitor());
    page.querySelector("#add-monitor-template").addEventListener("click", () => openTemplate());
    page.querySelectorAll("[data-monitor-action]").forEach(button => button.addEventListener("click", handleMonitorAction));
    page.querySelectorAll("[data-template-action]").forEach(button => button.addEventListener("click", handleTemplateAction));
  }

  function templateTable() {
    if (!templates.length) return emptyState("暂无请求模板", "创建模板后可在多个监控之间复用请求快照", "", "");
    return `<div class="table-wrap"><table><thead><tr><th>模板</th><th>协议</th><th>请求头</th><th>请求体</th><th>关联监控</th><th></th></tr></thead><tbody>${templates.map(item => `<tr><td><span class="cell-main">${escapeHtml(item.name)}</span><span class="cell-sub">${escapeHtml(item.description || "")}</span></td><td>${escapeHtml(item.provider.toUpperCase())}<span class="cell-sub">${item.api_mode === "responses" ? "Responses" : "Chat Completions"}</span></td><td>${Object.keys(item.extra_headers || {}).length} 个</td><td>${overrideLabel(item.body_override_mode)}</td><td>${item.associated_monitors}</td><td><div class="cell-actions"><button class="button quiet small" data-template-action="apply" data-id="${item.id}">应用</button><button class="button quiet small" data-template-action="edit" data-id="${item.id}">编辑</button><button class="button quiet small" data-template-action="delete" data-id="${item.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function monitorTable() {
    if (!monitors.length) return emptyState("暂无频道监控", "创建独立模型探测并记录可用率", "", "");
    const templateNames = new Map(templates.map(item => [item.id, item.name]));
    return `<div class="table-wrap"><table><thead><tr><th>名称</th><th>协议 / 模板</th><th>主模型</th><th>状态</th><th>7 天可用率</th><th>业务 / Ping</th><th>周期</th><th></th></tr></thead><tbody>${monitors.map(item => `<tr><td><span class="cell-main">${escapeHtml(item.name)}</span><span class="cell-sub">${escapeHtml(item.group_name || item.endpoint)}</span></td><td>${escapeHtml(item.provider.toUpperCase())}<span class="cell-sub">${item.template_id ? escapeHtml(templateNames.get(item.template_id) || `模板 #${item.template_id}`) : "手动快照"}</span></td><td><span class="cell-main mono">${escapeHtml(item.primary_model)}</span><span class="cell-sub">${item.extra_models.length} 个附加模型</span></td><td>${monitorStatus(item.primary_status)}${item.enabled ? "" : '<span class="cell-sub">监控已停用</span>'}</td><td>${Number(item.availability_7d).toFixed(2)}%</td><td>${latencyPair(item.primary_latency_ms, item.primary_ping_latency_ms)}</td><td>${item.interval_seconds} 秒</td><td><div class="cell-actions"><button class="button quiet small" data-monitor-action="run" data-id="${item.id}">运行</button><button class="button quiet small" data-monitor-action="history" data-id="${item.id}">历史</button><button class="button quiet small" data-monitor-action="duplicate" data-id="${item.id}">复制</button><button class="button quiet small" data-monitor-action="edit" data-id="${item.id}">编辑</button><button class="button quiet small" data-monitor-action="toggle" data-id="${item.id}">${item.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-monitor-action="delete" data-id="${item.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function latencyPair(request, ping) {
    if (request == null && ping == null) return "-";
    return `<span class="cell-main">${request == null ? "-" : `${request} ms`}</span><span class="cell-sub">Ping ${ping == null ? "-" : `${ping} ms`}</span>`;
  }

  function overrideLabel(mode) {
    return mode === "replace" ? "替换" : mode === "merge" ? "合并" : "关闭";
  }

  function templateOptions(selected) {
    return `<option value="" ${!selected ? "selected" : ""}>手动配置</option>${templates.map(item => `<option value="${item.id}" ${String(selected) === String(item.id) ? "selected" : ""}>${escapeHtml(item.name)} · ${escapeHtml(item.provider.toUpperCase())} · ${item.api_mode === "responses" ? "Responses" : "Chat"}</option>`).join("")}`;
  }

  function openMonitor(item = null) {
    openModal(item ? "编辑频道监控" : "创建频道监控", `<form id="advanced-monitor-form">
      <div class="form-grid"><div class="field"><label>名称</label><input name="name" value="${escapeHtml(item?.name || "")}" maxlength="100" required autofocus></div><div class="field"><label>分组</label><input name="group_name" value="${escapeHtml(item?.group_name || "")}" maxlength="80"></div></div>
      <div class="field"><label>请求模板</label><select name="template_id">${templateOptions(item?.template_id)}</select><span class="field-hint">选择模板会复制当前快照；后续模板修改需手动应用</span></div>
      <div class="form-grid"><div class="field"><label>提供方</label><select name="provider">${["openai", "anthropic", "gemini", "grok"].map(value => `<option value="${value}" ${item?.provider === value ? "selected" : ""}>${value.toUpperCase()}</option>`).join("")}</select></div><div class="field"><label>API 模式</label><select name="api_mode"><option value="chat_completions" ${item?.api_mode !== "responses" ? "selected" : ""}>Chat Completions</option><option value="responses" ${item?.api_mode === "responses" ? "selected" : ""}>Responses</option></select></div></div>
      <div class="field"><label>探测端点</label><input name="endpoint" type="url" value="${escapeHtml(item?.endpoint || "https://api.openai.com/v1/chat/completions")}" required></div>
      <div class="field"><label>API Key</label><input name="api_key" type="password" ${item ? "" : "required"} autocomplete="new-password"><span class="field-hint">${item ? `留空保留 ${escapeHtml(item.api_key_masked)}` : "加密保存，不会在列表或日志中返回"}</span></div>
      <div class="field"><label>主模型</label><input name="primary_model" value="${escapeHtml(item?.primary_model || "")}" required></div>
      <div class="field"><label>附加模型</label><textarea name="extra_models" class="compact-textarea" placeholder="每行一个模型">${escapeHtml((item?.extra_models || []).join("\n"))}</textarea></div>
      <div class="form-grid"><div class="field"><label>检查周期（秒）</label><input name="interval_seconds" type="number" min="30" max="86400" value="${item?.interval_seconds || 300}" required></div><div class="field"><label>随机偏移（秒）</label><input name="jitter_seconds" type="number" min="0" max="3600" value="${item?.jitter_seconds || 0}" required></div></div>
      ${snapshotFields(item)}
      <label class="switch-row"><span><strong>启用定时监控</strong><small>同一 Rust 进程按周期执行</small></span><input name="enabled" type="checkbox" ${item?.enabled === false ? "" : "checked"}></label>
      <p class="form-error" id="advanced-monitor-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-advanced-monitor">保存</button>');
    const form = modal.querySelector("#advanced-monitor-form");
    form.elements.template_id.addEventListener("change", event => {
      const template = templates.find(value => String(value.id) === event.currentTarget.value);
      if (!template) return;
      form.elements.provider.value = template.provider;
      form.elements.api_mode.value = template.api_mode;
      form.elements.extra_headers.value = JSON.stringify(template.extra_headers || {}, null, 2);
      form.elements.body_override_mode.value = template.body_override_mode;
      form.elements.body_override.value = template.body_override ? JSON.stringify(template.body_override, null, 2) : "";
    });
    const clearMismatchedTemplate = () => {
      const template = templates.find(value => String(value.id) === form.elements.template_id.value);
      if (template && (template.provider !== form.elements.provider.value || template.api_mode !== form.elements.api_mode.value)) form.elements.template_id.value = "";
    };
    form.elements.provider.addEventListener("change", clearMismatchedTemplate);
    form.elements.api_mode.addEventListener("change", clearMismatchedTemplate);
    modal.querySelector("#save-advanced-monitor").addEventListener("click", () => saveMonitor(item?.id));
  }

  function snapshotFields(item) {
    return `<div class="field"><label>附加请求头（JSON）</label><textarea name="extra_headers" class="compact-textarea" spellcheck="false">${escapeHtml(JSON.stringify(item?.extra_headers || {}, null, 2))}</textarea></div>
      <div class="field"><label>请求体覆盖</label><select name="body_override_mode"><option value="off" ${!item || item.body_override_mode === "off" ? "selected" : ""}>关闭</option><option value="merge" ${item?.body_override_mode === "merge" ? "selected" : ""}>合并</option><option value="replace" ${item?.body_override_mode === "replace" ? "selected" : ""}>替换</option></select></div>
      <div class="field"><label>请求体覆盖（JSON 对象）</label><textarea name="body_override" class="compact-textarea" spellcheck="false">${item?.body_override ? escapeHtml(JSON.stringify(item.body_override, null, 2)) : ""}</textarea></div>`;
  }

  async function saveMonitor(id) {
    const form = modal.querySelector("#advanced-monitor-form");
    if (!form.reportValidity()) return;
    const button = modal.querySelector("#save-advanced-monitor");
    button.disabled = true;
    try {
      const values = Object.fromEntries(new FormData(form));
      values.extra_models = parseModelList(values.extra_models);
      values.interval_seconds = Number(values.interval_seconds);
      values.jitter_seconds = Number(values.jitter_seconds);
      values.enabled = form.elements.enabled.checked;
      values.template_id = values.template_id ? Number(values.template_id) : null;
      values.extra_headers = JSON.parse(values.extra_headers || "{}");
      values.body_override = values.body_override.trim() ? JSON.parse(values.body_override) : null;
      if (id && !values.api_key) delete values.api_key;
      await api(id ? `/api/admin/channel-monitors/${id}` : "/api/admin/channel-monitors", { method: id ? "PUT" : "POST", body: JSON.stringify(values) });
      closeModal(); toast(id ? "监控已更新" : "监控已创建"); await renderRoute();
    } catch (error) { modal.querySelector("#advanced-monitor-error").textContent = error.message || "JSON 格式无效"; button.disabled = false; }
  }

  function openTemplate(item = null) {
    openModal(item ? "编辑请求模板" : "创建请求模板", `<form id="monitor-template-form">
      <div class="field"><label>名称</label><input name="name" value="${escapeHtml(item?.name || "")}" maxlength="100" required autofocus></div>
      <div class="form-grid"><div class="field"><label>提供方</label><select name="provider" ${item ? "disabled" : ""}>${["openai", "anthropic", "gemini", "grok"].map(value => `<option value="${value}" ${item?.provider === value ? "selected" : ""}>${value.toUpperCase()}</option>`).join("")}</select></div><div class="field"><label>API 模式</label><select name="api_mode"><option value="chat_completions" ${item?.api_mode !== "responses" ? "selected" : ""}>Chat Completions</option><option value="responses" ${item?.api_mode === "responses" ? "selected" : ""}>Responses</option></select></div></div>
      <div class="field"><label>说明</label><textarea name="description" class="compact-textarea" maxlength="500">${escapeHtml(item?.description || "")}</textarea></div>
      ${snapshotFields(item)}
      <p class="form-error" id="monitor-template-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-monitor-template">保存</button>');
    modal.querySelector("#save-monitor-template").addEventListener("click", () => saveTemplate(item));
  }

  async function saveTemplate(item) {
    const form = modal.querySelector("#monitor-template-form");
    if (!form.reportValidity()) return;
    const button = modal.querySelector("#save-monitor-template");
    button.disabled = true;
    try {
      const values = Object.fromEntries(new FormData(form));
      values.provider = form.elements.provider.value;
      values.extra_headers = JSON.parse(values.extra_headers || "{}");
      values.body_override = values.body_override.trim() ? JSON.parse(values.body_override) : null;
      if (item) delete values.provider;
      await api(item ? `/api/admin/channel-monitor-templates/${item.id}` : "/api/admin/channel-monitor-templates", { method: item ? "PUT" : "POST", body: JSON.stringify(values) });
      closeModal(); toast("请求模板已保存"); await renderRoute();
    } catch (error) { modal.querySelector("#monitor-template-error").textContent = error.message || "JSON 格式无效"; button.disabled = false; }
  }

  async function handleMonitorAction(event) {
    const button = event.currentTarget;
    const item = monitors.find(value => String(value.id) === button.dataset.id);
    if (!item) return;
    const action = button.dataset.monitorAction;
    if (action === "edit") return openMonitor(item);
    if (action === "delete") return confirmMonitorDelete(item);
    button.disabled = true;
    try {
      if (action === "run") {
        const result = await api(`/api/admin/channel-monitors/${item.id}/run`, { method: "POST", body: "{}" });
        openModal(`${item.name} · 探测结果`, historyTable(result.data.results), '<button class="button" data-close-modal>关闭</button>'); return;
      }
      if (action === "history") {
        const result = await api(`/api/admin/channel-monitors/${item.id}/history?limit=100`);
        openModal(`${item.name} · 历史`, result.data.length ? historyTable(result.data) : emptyState("暂无探测历史", "运行一次监控后会显示结果"), '<button class="button" data-close-modal>关闭</button>'); return;
      }
      if (action === "duplicate") await api(`/api/admin/channel-monitors/${item.id}/duplicate`, { method: "POST", body: "{}" });
      if (action === "toggle") await api(`/api/admin/channel-monitors/${item.id}`, { method: "PUT", body: JSON.stringify({ enabled: !item.enabled }) });
      toast(action === "duplicate" ? "监控副本已创建并默认停用" : "监控状态已更新"); await renderRoute();
    } catch (error) { toast(error.message, true); }
    finally { button.disabled = false; }
  }

  function confirmMonitorDelete(item) {
    openModal("删除频道监控", `<p>确认删除 <strong>${escapeHtml(item.name)}</strong> 及全部历史？</p><p class="form-error" id="monitor-delete-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-delete-monitor">删除</button>');
    modal.querySelector("#confirm-delete-monitor").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try { await api(`/api/admin/channel-monitors/${item.id}`, { method: "DELETE" }); closeModal(); toast("监控已删除"); await renderRoute(); }
      catch (error) { modal.querySelector("#monitor-delete-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function handleTemplateAction(event) {
    const item = templates.find(value => String(value.id) === event.currentTarget.dataset.id);
    if (!item) return;
    const action = event.currentTarget.dataset.templateAction;
    if (action === "edit") return openTemplate(item);
    if (action === "apply") return openApplyTemplate(item);
    openModal("删除请求模板", `<p>删除 <strong>${escapeHtml(item.name)}</strong>？关联监控会保留已复制的请求快照。</p><p class="form-error" id="template-delete-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-delete-template">删除</button>');
    modal.querySelector("#confirm-delete-template").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try { await api(`/api/admin/channel-monitor-templates/${item.id}`, { method: "DELETE" }); closeModal(); toast("模板已删除，监控快照已保留"); await renderRoute(); }
      catch (error) { modal.querySelector("#template-delete-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function openApplyTemplate(item) {
    const result = await api(`/api/admin/channel-monitor-templates/${item.id}/monitors`);
    const associated = result.data;
    openModal(`应用 ${item.name}`, associated.length ? `<p>选择要刷新为模板当前快照的关联监控。</p><div class="choice-grid">${associated.map(monitor => `<label><input type="checkbox" name="template_monitor_id" value="${monitor.id}" checked><span>${escapeHtml(monitor.name)}</span><small>${escapeHtml(monitor.provider.toUpperCase())} · ${monitor.enabled ? "启用" : "停用"}</small></label>`).join("")}</div><p class="form-error" id="template-apply-error"></p>` : emptyState("没有关联监控", "先在监控编辑器中选择此模板"), associated.length ? '<button class="button secondary" data-close-modal>取消</button><button class="button" id="confirm-apply-template">应用快照</button>' : '<button class="button" data-close-modal>关闭</button>');
    modal.querySelector("#confirm-apply-template")?.addEventListener("click", async event => {
      const monitor_ids = [...modal.querySelectorAll('[name="template_monitor_id"]:checked')].map(input => Number(input.value));
      if (!monitor_ids.length) { modal.querySelector("#template-apply-error").textContent = "至少选择一个监控"; return; }
      event.currentTarget.disabled = true;
      try { const applied = await api(`/api/admin/channel-monitor-templates/${item.id}/apply`, { method: "POST", body: JSON.stringify({ monitor_ids }) }); closeModal(); toast(`已刷新 ${applied.data.affected} 个监控`); await renderRoute(); }
      catch (error) { modal.querySelector("#template-apply-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  function historyTable(rows) {
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>模型</th><th>状态</th><th>业务延迟</th><th>Origin Ping</th><th>说明</th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.checked_at)}</td><td class="mono">${escapeHtml(row.model)}</td><td>${monitorStatus(row.status)}</td><td>${row.latency_ms == null ? "-" : `${row.latency_ms} ms`}</td><td>${row.ping_latency_ms == null ? "-" : `${row.ping_latency_ms} ms`}</td><td>${escapeHtml(row.message || "-")}</td></tr>`).join("")}</tbody></table></div>`;
  }

  window.Sub2MiniMonitorAdmin = { render };
})();
