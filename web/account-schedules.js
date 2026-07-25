"use strict";

window.Sub2MiniAccountSchedules = (() => {
  let activeAccount = null;
  let plans = [];
  let editing = null;
  let utcOffset = "+08:00";
  const results = new Map();

  function attach(page) {
    page.querySelectorAll("[data-account-schedules]").forEach(button => button.addEventListener("click", () => {
      const account = currentAccounts.find(item => String(item.id) === button.dataset.accountSchedules);
      if (!account) return;
      closeUpstreamAccountMenu();
      open(account);
    }));
  }

  async function open(account) {
    activeAccount = account;
    editing = null;
    results.clear();
    openModal(`${account.name} · 定时测试`, `<div class="boot-screen"><p>正在载入</p></div>`, `<button class="button" data-close-modal>关闭</button>`);
    await load();
  }

  async function load() {
    try {
      const response = await api(`/api/admin/accounts/${activeAccount.id}/scheduled-test-plans`);
      plans = response.data;
      utcOffset = response.meta?.utc_offset || utcOffset;
      render();
    } catch (error) {
      closeModal();
      toast(error.message, true);
    }
  }

  function render() {
    const body = modal.querySelector(".modal-body");
    body.innerHTML = `<div class="schedule-toolbar"><span>UTC ${escapeHtml(utcOffset)}</span><button class="button small" data-schedule-action="add">添加计划</button></div>
      ${editing ? form(editing) : ""}
      <div class="schedule-list">${plans.length ? plans.map(planCard).join("") : emptyState("暂无定时测试", "添加计划后将按设定时间检查账号连通性")}</div>`;
    body.querySelectorAll("[data-schedule-action]").forEach(button => button.addEventListener("click", handleAction));
  }

  function form(value) {
    const isNew = value.id == null;
    return `<form class="schedule-form" id="scheduled-test-form">
      <div class="section-title"><h2>${isNew ? "添加测试计划" : "编辑测试计划"}</h2></div>
      <div class="form-grid"><div class="field"><label for="schedule-model">模型</label><input id="schedule-model" name="model_id" maxlength="100" value="${escapeHtml(value.model_id || "gpt-5")}" required></div><div class="field"><label for="schedule-cron">Cron</label><input id="schedule-cron" name="cron_expression" maxlength="100" value="${escapeHtml(value.cron_expression || "*/30 * * * *")}" required></div></div>
      <div class="form-grid"><div class="field"><label for="schedule-retention">保留结果</label><input id="schedule-retention" name="max_results" type="number" min="1" max="500" value="${Number(value.max_results || 50)}" required></div><div class="schedule-checks"><label class="toggle-line"><input name="enabled" type="checkbox" ${value.enabled === false ? "" : "checked"}> 启用</label><label class="toggle-line"><input name="auto_recover" type="checkbox" ${value.auto_recover ? "checked" : ""}> 成功后自动恢复</label></div></div>
      <p class="form-error" id="schedule-form-error"></p><div class="form-actions"><button class="button secondary" type="button" data-schedule-action="cancel">取消</button><button class="button" type="button" data-schedule-action="save">保存</button></div>
    </form>`;
  }

  function planCard(plan) {
    const history = results.get(plan.id);
    return `<article class="schedule-item">
      <div class="schedule-item-head"><div><strong>${escapeHtml(plan.model_id)}</strong><code>${escapeHtml(plan.cron_expression)}</code></div><span class="status ${plan.enabled ? "" : "off"}">${plan.enabled ? "运行中" : "已停用"}</span></div>
      <div class="schedule-meta"><span>下次 ${formatDate(plan.next_run_at)}</span><span>上次 ${formatDate(plan.last_run_at)}</span><span>保留 ${formatNumber(plan.max_results)}</span>${plan.auto_recover ? "<span>自动恢复</span>" : ""}</div>
      <div class="cell-actions schedule-actions"><button class="button quiet small" data-schedule-action="run" data-id="${plan.id}">立即测试</button><button class="button quiet small" data-schedule-action="results" data-id="${plan.id}">${history ? "收起结果" : "查看结果"}</button><button class="button quiet small" data-schedule-action="toggle" data-id="${plan.id}">${plan.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-schedule-action="edit" data-id="${plan.id}">编辑</button><button class="button quiet small danger" data-schedule-action="delete" data-id="${plan.id}">删除</button></div>
      ${history ? resultList(history) : ""}
    </article>`;
  }

  function resultList(items) {
    if (!items.length) return `<div class="schedule-results"><p class="muted">暂无测试结果</p></div>`;
    return `<div class="schedule-results">${items.map(item => `<div class="schedule-result"><div><span class="status ${item.status === "success" ? "" : "warn"}">${item.status === "success" ? "成功" : "失败"}</span><strong>${formatNumber(item.latency_ms)} ms</strong><time>${formatDate(item.started_at)}</time></div>${item.error_message ? `<pre>${escapeHtml(item.error_message)}</pre>` : `<pre>${escapeHtml(item.response_text || "已通过连通性检查")}</pre>`}</div>`).join("")}</div>`;
  }

  async function handleAction(event) {
    const button = event.currentTarget;
    const action = button.dataset.scheduleAction;
    if (action === "add") { editing = { enabled: true, auto_recover: false, max_results: 50, model_id: "gpt-5", cron_expression: "*/30 * * * *" }; return render(); }
    if (action === "cancel") { editing = null; return render(); }
    if (action === "save") return save(button);
    const id = Number(button.dataset.id);
    const plan = plans.find(item => item.id === id);
    if (!plan) return;
    if (action === "edit") { editing = { ...plan }; return render(); }
    if (action === "results") {
      if (results.has(id)) { results.delete(id); return render(); }
      button.disabled = true;
      try { results.set(id, (await api(`/api/admin/scheduled-test-plans/${id}/results?limit=20`)).data); render(); }
      catch (error) { toast(error.message, true); button.disabled = false; }
      return;
    }
    if (action === "delete" && !confirm(`删除 ${plan.model_id} 的定时测试计划及历史结果？`)) return;
    button.disabled = true;
    try {
      if (action === "run") {
        const response = await api(`/api/admin/scheduled-test-plans/${id}/run`, { method: "POST", body: "{}" });
        toast(response.data.status === "success" ? "账号测试成功" : `账号测试失败：${response.data.error_message}`, response.data.status !== "success");
        results.set(id, (await api(`/api/admin/scheduled-test-plans/${id}/results?limit=20`)).data);
      } else if (action === "toggle") {
        await api(`/api/admin/scheduled-test-plans/${id}`, { method: "PUT", body: JSON.stringify({ enabled: !plan.enabled }) });
        toast(plan.enabled ? "测试计划已停用" : "测试计划已启用");
      } else if (action === "delete") {
        await api(`/api/admin/scheduled-test-plans/${id}`, { method: "DELETE" });
        results.delete(id);
        toast("测试计划已删除");
      }
      await load();
    } catch (error) { toast(error.message, true); button.disabled = false; }
  }

  async function save(button) {
    const formElement = modal.querySelector("#scheduled-test-form");
    if (!formElement.reportValidity()) return;
    const values = new FormData(formElement);
    const payload = { model_id: values.get("model_id"), cron_expression: values.get("cron_expression"), max_results: Number(values.get("max_results")), enabled: values.has("enabled"), auto_recover: values.has("auto_recover") };
    if (!editing.id) payload.account_id = activeAccount.id;
    button.disabled = true;
    try {
      await api(editing.id ? `/api/admin/scheduled-test-plans/${editing.id}` : "/api/admin/scheduled-test-plans", { method: editing.id ? "PUT" : "POST", body: JSON.stringify(payload) });
      toast(editing.id ? "测试计划已更新" : "测试计划已创建"); editing = null; await load();
    } catch (error) { modal.querySelector("#schedule-form-error").textContent = error.message; button.disabled = false; }
  }

  return { attach };
})();
