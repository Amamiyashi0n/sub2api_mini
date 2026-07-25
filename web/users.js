"use strict";

(() => {
  let users = [];
  let selectedUserIds = new Set();

  async function render(page) {
    const result = await api("/api/admin/users");
    users = result.data;
    selectedUserIds = new Set();
    const regularUsers = users.filter(user => user.role === "user").length;
    page.innerHTML = `
      ${pageHeader("用户管理", `${regularUsers} 个普通用户`, `<button class="button" id="add-user">创建用户</button>`)}
      <div class="inline-filters user-filters"><div class="field"><label for="user-search">搜索</label><input id="user-search" type="search" placeholder="用户名、显示名称或邮箱"></div><div class="field"><label for="user-status-filter">状态</label><select id="user-status-filter"><option value="">全部</option><option value="enabled">正常</option><option value="disabled">停用</option></select></div><div class="field"><label for="user-role-filter">角色</label><select id="user-role-filter"><option value="">全部</option><option value="user">普通用户</option><option value="admin">管理员</option></select></div></div>
      <div class="user-batch-bar"><span id="user-selection-count">未选择用户</span><select id="user-batch-action" aria-label="批量操作"><option value="enable">启用</option><option value="disable">停用</option><option value="delete">删除</option></select><button class="button secondary" id="apply-user-batch" disabled>应用</button></div>
      <div id="user-list"></div>`;
    page.querySelector("#add-user").addEventListener("click", openCreateUser);
    ["#user-search", "#user-status-filter", "#user-role-filter"].forEach(selector => page.querySelector(selector).addEventListener("input", () => updateList(page)));
    page.querySelector("#apply-user-batch").addEventListener("click", applyBatch);
    updateList(page);
  }

  function updateList(page) {
    const query = page.querySelector("#user-search").value.trim().toLowerCase();
    const statusFilter = page.querySelector("#user-status-filter").value;
    const roleFilter = page.querySelector("#user-role-filter").value;
    const visible = users.filter(user => {
      const searchable = [user.username, user.display_name, user.email].filter(Boolean).join(" ").toLowerCase();
      return (!query || searchable.includes(query))
        && (!statusFilter || (statusFilter === "enabled") === Boolean(user.enabled))
        && (!roleFilter || user.role === roleFilter);
    });
    const container = page.querySelector("#user-list");
    container.innerHTML = visible.length ? userTable(visible) : emptyState("没有匹配的用户", "调整搜索或筛选条件");
    container.querySelectorAll("[data-user-action]").forEach(button => button.addEventListener("click", handleAction));
    container.querySelectorAll("[data-user-select]").forEach(input => input.addEventListener("change", event => {
      const id = Number(event.currentTarget.value);
      event.currentTarget.checked ? selectedUserIds.add(id) : selectedUserIds.delete(id);
      updateBatchState(page, visible);
    }));
    const selectAll = container.querySelector("#user-select-all");
    const selectable = visible.filter(user => user.role === "user");
    if (selectAll) {
      selectAll.checked = selectable.length > 0 && selectable.every(user => selectedUserIds.has(user.id));
      selectAll.addEventListener("change", event => {
        selectable.forEach(user => event.currentTarget.checked ? selectedUserIds.add(user.id) : selectedUserIds.delete(user.id));
        updateList(page);
      });
    }
    updateBatchState(page, selectable);
  }

  function userTable(rows) {
    return `<div class="table-wrap"><table class="user-table"><thead><tr><th><input type="checkbox" id="user-select-all" aria-label="选择当前用户"></th><th>用户</th><th>邮箱</th><th>角色 / 状态</th><th>余额</th><th>累计使用</th><th>订阅 / Key</th><th class="hide-mobile">最后请求</th><th></th></tr></thead><tbody>${rows.map(user => `<tr><td>${user.role === "user" ? `<input type="checkbox" data-user-select value="${user.id}" aria-label="选择 ${escapeHtml(user.username)}" ${selectedUserIds.has(user.id) ? "checked" : ""}>` : ""}</td><td><span class="cell-main">${escapeHtml(user.display_name)}</span><span class="cell-sub mono">${escapeHtml(user.username)}</span>${user.notes ? `<span class="cell-sub">${escapeHtml(user.notes.slice(0, 60))}</span>` : ""}</td><td><span class="cell-main">${escapeHtml(user.email || "-")}</span>${user.email ? `<span class="cell-sub">${user.email_verified ? "已验证" : "未验证"}</span>` : ""}</td><td><span class="cell-main">${user.role === "admin" ? "管理员" : "普通用户"}</span>${user.enabled ? status("正常") : status("停用", "off")}</td><td>${formatMoney(user.balance_cents)}</td><td><span class="cell-main">${formatNumber(user.total_requests)} 请求</span><span class="cell-sub">${formatNumber(user.total_tokens)} Token · ${formatMicrousd(user.total_cost_microusd)}</span></td><td><span class="cell-main">${user.active_subscriptions} 个活跃订阅</span><span class="cell-sub">${user.key_count} 个 Key</span></td><td class="hide-mobile">${formatDate(user.last_request_at)}</td><td><div class="cell-actions"><button class="button quiet small" data-user-action="detail" data-id="${user.id}">详情</button>${user.role === "user" ? `<button class="button quiet small" data-user-action="groups" data-id="${user.id}">分组权限</button><button class="button quiet small" data-user-action="edit" data-id="${user.id}">编辑</button><button class="button quiet small" data-user-action="password" data-id="${user.id}">改密</button><button class="button quiet small" data-user-action="toggle" data-id="${user.id}" data-enabled="${user.enabled}">${user.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-user-action="delete" data-id="${user.id}">删除</button>` : ""}</div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function updateBatchState(page, visible = users.filter(user => user.role === "user")) {
    const count = page.querySelector("#user-selection-count");
    const button = page.querySelector("#apply-user-batch");
    count.textContent = selectedUserIds.size ? `已选择 ${selectedUserIds.size} 个用户` : "未选择用户";
    button.disabled = selectedUserIds.size === 0;
    const selectAll = page.querySelector("#user-select-all");
    if (selectAll) selectAll.checked = visible.length > 0 && visible.every(user => selectedUserIds.has(user.id));
  }

  async function applyBatch() {
    const action = document.querySelector("#user-batch-action").value;
    const ids = [...selectedUserIds];
    const labels = { enable: "启用", disable: "停用", delete: "删除" };
    if (!ids.length) return;
    if (["disable", "delete"].includes(action) && !confirm(`确认${labels[action]}所选 ${ids.length} 个用户？删除会撤销全部 Key 并保留财务历史。`)) return;
    const button = document.querySelector("#apply-user-batch");
    button.disabled = true;
    try {
      const result = await api("/api/admin/users/batch", { method: "POST", body: JSON.stringify({ ids, action }) });
      toast(`${labels[action]}完成，共处理 ${result.data.affected} 个用户`);
      await renderRoute();
    } catch (error) { toast(error.message, true); button.disabled = false; }
  }

  async function handleAction(event) {
    const button = event.currentTarget;
    const id = Number(button.dataset.id);
    const user = users.find(item => item.id === id);
    if (!user) return;
    const action = button.dataset.userAction;
    if (action === "detail") return openDetail(id);
    if (action === "groups") return openGroupAccess(user);
    if (action === "edit") return openEdit(user);
    if (action === "password") return openPassword(user);
    if (action === "delete" && !confirm("删除会撤销该用户全部 Key、取消活跃订阅并保留订单与余额历史，确认继续？")) return;
    button.disabled = true;
    try {
      if (action === "toggle") await api(`/api/admin/users/${id}`, { method: "PUT", body: JSON.stringify({ enabled: button.dataset.enabled !== "true" }) });
      else await api(`/api/admin/users/${id}`, { method: "DELETE" });
      toast(action === "delete" ? "用户已删除" : "用户状态已更新");
      await renderRoute();
    } catch (error) { toast(error.message, true); button.disabled = false; }
  }

  function openCreateUser() {
    compactModal();
    openModal("创建用户", `<form id="user-form"><div class="form-grid"><div class="field"><label for="new-username">用户名</label><input id="new-username" name="username" minlength="3" maxlength="64" pattern="[A-Za-z0-9._-]+" required autofocus></div><div class="field"><label for="display-name">显示名称</label><input id="display-name" name="display_name" maxlength="80"></div></div><div class="field"><label for="new-user-email">邮箱（可选）</label><input id="new-user-email" name="email" type="email" maxlength="254" autocomplete="email"></div><div class="form-grid"><div class="field"><label for="new-password">初始密码</label><input id="new-password" name="password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required></div><div class="field"><label for="initial-balance">初始余额 (CNY)</label><input id="initial-balance" name="balance_yuan" type="number" min="0" max="1000000000000" step="0.01" value="0"></div></div><div class="field"><label for="new-user-notes">管理员备注</label><textarea id="new-user-notes" name="notes" class="compact-textarea" maxlength="2000"></textarea></div><p class="form-error" id="user-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-user">创建</button>`);
    modal.querySelector("#save-user").addEventListener("click", saveCreateUser);
  }

  async function saveCreateUser() {
    const form = modal.querySelector("#user-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    values.email = values.email.trim() || null;
    values.balance_cents = yuanToCents(values.balance_yuan);
    delete values.balance_yuan;
    const button = modal.querySelector("#save-user");
    button.disabled = true;
    try {
      await api("/api/admin/users", { method: "POST", body: JSON.stringify(values) });
      closeModal(); toast("用户已创建"); await renderRoute();
    } catch (error) { modal.querySelector("#user-error").textContent = error.message; button.disabled = false; }
  }

  async function openDetail(id) {
    const result = await api(`/api/admin/users/${id}`);
    const data = result.data;
    const user = data.user;
    openModal(`用户详情 · ${user.username}`, `<div class="user-detail-summary"><div><span>余额</span><strong>${formatMoney(user.balance_cents)}</strong></div><div><span>累计请求</span><strong>${formatNumber(user.total_requests)}</strong></div><div><span>累计 Token</span><strong>${formatNumber(user.total_tokens)}</strong></div><div><span>累计成本</span><strong>${formatMicrousd(user.total_cost_microusd)}</strong></div></div><dl class="detail-list user-identity"><div><dt>显示名称</dt><dd>${escapeHtml(user.display_name)}</dd></div><div><dt>邮箱</dt><dd>${escapeHtml(user.email || "-")} ${user.email ? (user.email_verified ? status("已验证") : status("未验证", "warn")) : ""}</dd></div><div><dt>状态</dt><dd>${user.enabled ? status("正常") : status("停用", "off")}</dd></div><div><dt>备注</dt><dd>${escapeHtml(user.notes || "-")}</dd></div><div><dt>创建 / 最近请求</dt><dd>${formatDate(user.created_at)} / ${formatDate(user.last_request_at)}</dd></div></dl>${detailSection("API Key", keyRows(data.keys))}${detailSection("订阅", subscriptionRows(data.subscriptions))}${detailSection("订单", orderRows(data.orders))}${detailSection("余额流水", balanceRows(data.balance_adjustments))}${detailSection("近 30 天使用", trendRows(data.trend))}`, `<button class="button secondary" data-close-modal>关闭</button>${user.role === "user" ? `<button class="button secondary" id="detail-adjust-balance">调整余额</button><button class="button" id="detail-edit-user">编辑用户</button>` : ""}`);
    wideModal();
    modal.querySelector("#detail-adjust-balance")?.addEventListener("click", () => openBalance(user));
    modal.querySelector("#detail-edit-user")?.addEventListener("click", () => openEdit(user));
  }

  function detailSection(title, body) {
    return `<section class="user-detail-section"><div class="section-title"><h2>${escapeHtml(title)}</h2></div>${body}</section>`;
  }

  function keyRows(rows) {
    if (!rows.length) return emptyState("暂无 Key", "该用户尚未创建下游 Key");
    return `<div class="table-wrap"><table><thead><tr><th>名称</th><th>状态</th><th>分组</th><th>Token</th><th>消费</th><th>最后使用</th></tr></thead><tbody>${rows.map(row => `<tr><td><span class="cell-main">${escapeHtml(row.name)}</span><span class="cell-sub mono">${escapeHtml(row.token_prefix)}...</span></td><td>${row.status === "active" ? status("有效") : status(row.status, "warn")}</td><td>${escapeHtml(row.group_name || "全部账号")}</td><td>${formatNumber(row.used_tokens)} / ${row.quota_tokens ? formatNumber(row.quota_tokens) : "无限"}</td><td>${formatMicrousd(row.used_cost_microusd)} / ${row.quota_cost_microusd ? formatMicrousd(row.quota_cost_microusd) : "无限"}</td><td>${formatDate(row.last_used_at)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function subscriptionRows(rows) {
    if (!rows.length) return emptyState("暂无订阅", "可在套餐管理中为用户分配订阅");
    return `<div class="table-wrap"><table><thead><tr><th>套餐</th><th>状态</th><th>Token 用量</th><th>开始</th><th>结束</th></tr></thead><tbody>${rows.map(row => `<tr><td>${escapeHtml(row.plan_name)}</td><td>${row.status === "active" ? status("有效") : status(row.status, "off")}</td><td>${formatNumber(row.used_tokens)} / ${row.token_limit ? formatNumber(row.token_limit) : "无限"}</td><td>${formatDate(row.starts_at)}</td><td>${formatDate(row.ends_at)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function orderRows(rows) {
    if (!rows.length) return emptyState("暂无订单", "该用户没有订单记录");
    return `<div class="table-wrap"><table><thead><tr><th>订单</th><th>套餐</th><th>金额</th><th>状态</th><th>时间</th></tr></thead><tbody>${rows.map(row => `<tr><td class="mono">#${row.id}</td><td>${escapeHtml(row.plan_name)}</td><td>${formatMoney(row.amount_cents)}</td><td>${status(row.status, row.status === "paid" ? "" : "off")}</td><td>${formatDate(row.created_at)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function balanceRows(rows) {
    if (!rows.length) return emptyState("暂无余额流水", "管理员调整余额后会保留审计记录");
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>变化</th><th>调整后</th><th>原因</th><th>管理员</th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td><td class="mono ${row.delta_cents < 0 ? "text-danger" : ""}">${row.delta_cents > 0 ? "+" : ""}${formatMoney(row.delta_cents)}</td><td>${formatMoney(row.balance_after_cents)}</td><td>${escapeHtml(row.reason)}</td><td>${escapeHtml(row.admin_username || "system")}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function trendRows(rows) {
    if (!rows.length) return emptyState("暂无用量", "完成网关请求后会显示趋势");
    return `<div class="table-wrap"><table><thead><tr><th>日期</th><th>请求</th><th>Token</th><th>成本</th></tr></thead><tbody>${rows.map(row => `<tr><td>${escapeHtml(row.date)}</td><td>${formatNumber(row.requests)}</td><td>${formatNumber(row.tokens)}</td><td>${formatMicrousd(row.cost_microusd)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function openEdit(user) {
    compactModal();
    openModal("编辑用户", `<form id="user-edit-form"><div class="form-grid"><div class="field"><label for="edit-username">用户名</label><input id="edit-username" name="username" value="${escapeHtml(user.username)}" minlength="3" maxlength="64" pattern="[A-Za-z0-9._-]+" required autofocus></div><div class="field"><label for="edit-display-name">显示名称</label><input id="edit-display-name" name="display_name" value="${escapeHtml(user.display_name)}" maxlength="80" required></div></div><div class="field"><label for="edit-user-email">邮箱</label><input id="edit-user-email" name="email" type="email" value="${escapeHtml(user.email || "")}" maxlength="254"></div><div class="form-grid"><label class="switch-row compact"><span><strong>邮箱已验证</strong><small>管理员确认该邮箱归属</small></span><input name="email_verified" type="checkbox" ${user.email_verified ? "checked" : ""}></label><label class="switch-row compact"><span><strong>账户启用</strong><small>停用会撤销现有会话和 Key 访问</small></span><input name="enabled" type="checkbox" ${user.enabled ? "checked" : ""}></label></div><div class="field"><label for="edit-user-notes">管理员备注</label><textarea id="edit-user-notes" name="notes" class="compact-textarea" maxlength="2000">${escapeHtml(user.notes || "")}</textarea></div><p class="form-error" id="user-edit-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-user-edit">保存</button>`);
    modal.querySelector("#save-user-edit").addEventListener("click", () => saveEdit(user.id));
  }

  async function saveEdit(id) {
    const form = modal.querySelector("#user-edit-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    values.email = values.email.trim() || null;
    values.email_verified = form.elements.email_verified.checked;
    values.enabled = form.elements.enabled.checked;
    const button = modal.querySelector("#save-user-edit");
    button.disabled = true;
    try {
      await api(`/api/admin/users/${id}`, { method: "PUT", body: JSON.stringify(values) });
      closeModal(); toast("用户资料已更新"); await renderRoute();
    } catch (error) { modal.querySelector("#user-edit-error").textContent = error.message; button.disabled = false; }
  }

  function openPassword(user) {
    compactModal();
    openModal("重置用户密码", `<form id="password-form"><p class="field-hint mono">${escapeHtml(user.username)}</p><div class="field"><label for="reset-password">新密码</label><input id="reset-password" name="password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required autofocus></div><p class="form-error" id="password-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-password">保存</button>`);
    modal.querySelector("#save-password").addEventListener("click", async event => {
      const form = modal.querySelector("#password-form");
      if (!form.reportValidity()) return;
      event.currentTarget.disabled = true;
      try {
        await api(`/api/admin/users/${user.id}`, { method: "PUT", body: JSON.stringify(Object.fromEntries(new FormData(form))) });
        closeModal(); toast("密码已更新，原会话已失效");
      } catch (error) { modal.querySelector("#password-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function openGroupAccess(user) {
    try {
      const [accessResult, groupResult] = await Promise.all([
        api(`/api/admin/users/${user.id}/groups`),
        api("/api/admin/groups"),
      ]);
      const access = accessResult.data;
      const selected = new Set(access.allowed_group_ids || []);
      const standardGroups = groupResult.data.filter(group => group.subscription_type === "standard");
      openModal(`分组权限 · ${escapeHtml(user.username)}`, `<form id="user-groups-form">
        <label class="switch-row"><span><strong>允许全部公共标准分组</strong><small>专属分组始终需要显式授权；订阅分组由有效订阅控制</small></span><input id="allow-all-standard-groups" type="checkbox" ${access.allow_all_standard_groups ? "checked" : ""}></label>
        <div class="field"><label>显式授权</label><div class="choice-grid">${standardGroups.map(group => `<label><input type="checkbox" name="allowed_group_id" value="${group.id}" ${selected.has(group.id) ? "checked" : ""}><span>${escapeHtml(group.name)}</span><small>${group.is_exclusive ? "专属" : "公共"} · ${group.enabled ? "启用" : "停用"}</small></label>`).join("") || '<span class="field-hint">暂无标准分组</span>'}</div></div>
        <p class="form-error" id="user-groups-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-user-groups">保存</button>');
      modal.querySelector("#save-user-groups").addEventListener("click", async event => {
        event.currentTarget.disabled = true;
        const payload = {
          allow_all_standard_groups: modal.querySelector("#allow-all-standard-groups").checked,
          allowed_group_ids: [...modal.querySelectorAll('[name="allowed_group_id"]:checked')].map(input => Number(input.value)),
        };
        try {
          await api(`/api/admin/users/${user.id}/groups`, { method: "PUT", body: JSON.stringify(payload) });
          closeModal(); toast("用户分组权限已更新");
        } catch (error) { modal.querySelector("#user-groups-error").textContent = error.message; event.currentTarget.disabled = false; }
      });
    } catch (error) { toast(error.message, true); }
  }

  function openBalance(user) {
    compactModal();
    openModal("调整用户余额", `<form id="balance-form"><p>当前余额 <strong>${formatMoney(user.balance_cents)}</strong></p><div class="field"><label for="balance-delta">变化金额 (CNY)</label><input id="balance-delta" name="delta_yuan" type="number" step="0.01" required autofocus><span class="field-hint">正数增加，负数扣减</span></div><div class="field"><label for="balance-reason">调整原因</label><input id="balance-reason" name="reason" maxlength="200" required></div><p class="form-error" id="balance-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-balance">确认调整</button>`);
    modal.querySelector("#save-balance").addEventListener("click", async event => {
      const form = modal.querySelector("#balance-form");
      if (!form.reportValidity()) return;
      event.currentTarget.disabled = true;
      try {
        const delta_cents = yuanToCents(form.elements.delta_yuan.value, true);
        await api(`/api/admin/users/${user.id}/balance`, { method: "POST", body: JSON.stringify({ delta_cents, reason: form.elements.reason.value }) });
        closeModal(); toast("余额已调整并记录流水"); await renderRoute();
      } catch (error) { modal.querySelector("#balance-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  function yuanToCents(value, allowNegative = false) {
    const amount = Number(value || 0);
    if (!Number.isFinite(amount) || (!allowNegative && amount < 0) || Math.abs(amount) > 1000000000000) throw new Error("金额超出支持范围");
    const cents = Math.round(amount * 100);
    if (allowNegative && cents === 0) throw new Error("调整金额不能为零");
    return cents;
  }

  function wideModal() {
    modal.classList.add("user-detail-modal");
    modal.addEventListener("close", compactModal, { once: true });
  }

  function compactModal() { modal.classList.remove("user-detail-modal"); }

  window.Sub2MiniUsers = { render };
})();
