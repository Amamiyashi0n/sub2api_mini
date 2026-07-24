"use strict";

window.Sub2MiniEngagement = (() => {
  let announcementPage = 1;
  let announcementFilters = { search: "", status: "" };
  let announcementItems = [];
  let contentPages = [];
  let plans = [];
  let editingTargeting = { any_of: [] };
  let popupQueue = [];
  let popupShowing = false;
  let popupFetchedAt = 0;
  const popupStorageKey = "mini_shown_announcement_popups";
  let shownPopups = new Set(JSON.parse(sessionStorage.getItem(popupStorageKey) || "[]"));

  function pager(kind, meta) {
    const pages = Math.max(1, Math.ceil(meta.total / meta.page_size));
    if (pages <= 1) return "";
    return `<div class="pagination"><button class="button secondary small" data-engagement-page="${kind}" data-direction="prev" ${meta.page <= 1 ? "disabled" : ""}>上一页</button><span>${meta.page} / ${pages}</span><button class="button secondary small" data-engagement-page="${kind}" data-direction="next" ${meta.page >= pages ? "disabled" : ""}>下一页</button></div>`;
  }

  async function renderAnnouncements(page) {
    const result = await api("/api/user/announcements");
    const unread = result.data.filter(item => !item.is_read);
    page.innerHTML = `${pageHeader("公告", `${unread.length} 条未读`, `<button class="button secondary" id="refresh-announcements">刷新</button>${unread.length ? '<button class="button" id="read-all-announcements">全部已读</button>' : ""}`)}${result.data.length ? announcementCards(result.data) : emptyState("暂无公告", "发布后的公告会显示在这里")}`;
    page.querySelector("#refresh-announcements")?.addEventListener("click", renderRoute);
    page.querySelector("#read-all-announcements")?.addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try { await Promise.all(unread.map(item => api(`/api/user/announcements/${item.id}/read`, { method: "POST", body: "{}" }))); toast("公告已全部标记为已读"); await renderRoute(); }
      catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
    });
    page.querySelectorAll("[data-read-announcement]").forEach(button => button.addEventListener("click", markAnnouncementRead));
  }

  async function markAnnouncementRead(event) {
    event.currentTarget.disabled = true;
    try { await api(`/api/user/announcements/${event.currentTarget.dataset.readAnnouncement}/read`, { method: "POST", body: "{}" }); await renderRoute(); }
    catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
  }

  function targetingSummary(targeting) {
    const groups = targeting?.any_of || [];
    if (!groups.length) return "全部用户";
    const conditions = groups.reduce((total, group) => total + (group.all_of || []).length, 0);
    return `${groups.length} 组 / ${conditions} 条规则`;
  }

  async function renderContentAdmin(page) {
    const query = new URLSearchParams({ page: String(announcementPage), page_size: "20", sort_by: "created_at", sort_order: "desc" });
    Object.entries(announcementFilters).forEach(([key, value]) => { if (value) query.set(key, value); });
    const [announcementResult, pageResult, planResult] = await Promise.all([api(`/api/admin/announcements?${query}`), api("/api/admin/pages"), api("/api/admin/plans")]);
    announcementItems = announcementResult.data; contentPages = pageResult.data; plans = planResult.data;
    page.innerHTML = `
      ${pageHeader("内容管理", "公告、法律文档与自定义页面")}
      <section><div class="section-title"><h2>公告</h2><button class="button" id="add-announcement">创建公告</button></div>
        <form id="announcement-filter-form" class="filter-bar engagement-filter"><div class="field"><label for="announcement-search">标题或内容</label><input id="announcement-search" name="search" type="search" value="${escapeHtml(announcementFilters.search)}"></div><div class="field"><label for="announcement-filter-status">状态</label><select id="announcement-filter-status" name="status"><option value="">全部</option><option value="draft" ${announcementFilters.status === "draft" ? "selected" : ""}>草稿</option><option value="active" ${announcementFilters.status === "active" ? "selected" : ""}>发布</option><option value="archived" ${announcementFilters.status === "archived" ? "selected" : ""}>归档</option></select></div><div class="filter-actions"><button class="button" type="submit">筛选</button><button class="button quiet" type="button" id="clear-announcement-filter">清除</button></div></form>
        ${announcementItems.length ? announcementAdminTable(announcementItems) : emptyState("暂无公告", "创建后可发布到用户控制台", "创建公告", "empty-add-announcement")}${pager("announcements", announcementResult.meta)}</section>
      <section class="section"><div class="section-title"><h2>内容页</h2><button class="button" id="add-content-page">创建内容页</button></div>${contentPages.length ? pageAdminTable(contentPages) : emptyState("暂无内容页", "可创建法律文档或自定义内容", "创建内容页", "empty-add-content-page")}</section>`;
    page.querySelector("#add-announcement")?.addEventListener("click", () => openAnnouncementEditor());
    page.querySelector("#empty-add-announcement")?.addEventListener("click", () => openAnnouncementEditor());
    page.querySelector("#add-content-page")?.addEventListener("click", () => openContentPageEditor());
    page.querySelector("#empty-add-content-page")?.addEventListener("click", () => openContentPageEditor());
    page.querySelector("#announcement-filter-form")?.addEventListener("submit", event => { event.preventDefault(); announcementFilters = Object.fromEntries(new FormData(event.currentTarget)); announcementPage = 1; renderRoute(); });
    page.querySelector("#clear-announcement-filter")?.addEventListener("click", () => { announcementFilters = { search: "", status: "" }; announcementPage = 1; renderRoute(); });
    page.querySelectorAll("[data-announcement-action]").forEach(button => button.addEventListener("click", handleAnnouncementAction));
    page.querySelectorAll("[data-page-action]").forEach(button => button.addEventListener("click", handlePageAction));
    page.querySelectorAll('[data-engagement-page="announcements"]').forEach(button => button.addEventListener("click", () => { announcementPage = Math.max(1, announcementPage + (button.dataset.direction === "next" ? 1 : -1)); renderRoute(); }));
  }

  function announcementAdminTable(items) {
    return `<div class="table-wrap"><table class="announcement-admin-table"><thead><tr><th>标题</th><th>状态</th><th>通知</th><th>受众</th><th>展示时间</th><th>已读</th><th></th></tr></thead><tbody>${items.map(item => `<tr><td><span class="cell-main">${escapeHtml(item.title)}</span><span class="cell-sub">${formatDate(item.created_at)}</span></td><td>${item.status === "active" ? status("已发布") : item.status === "archived" ? status("已归档", "off") : status("草稿", "warn")}</td><td>${item.notify_mode === "popup" ? status("弹窗", "warn") : status("静默", "off")}</td><td>${escapeHtml(targetingSummary(item.targeting))}</td><td><span class="cell-sub">${item.starts_at ? formatDate(item.starts_at) : "立即"} - ${item.ends_at ? formatDate(item.ends_at) : "长期"}</span></td><td>${formatNumber(item.read_count)}</td><td><div class="cell-actions"><button class="button quiet small" data-announcement-action="read" data-id="${item.id}">已读状态</button><button class="button quiet small" data-announcement-action="edit" data-id="${item.id}">编辑</button><button class="button quiet small" data-announcement-action="delete" data-id="${item.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  async function handleAnnouncementAction(event) {
    const item = announcementItems.find(row => row.id === Number(event.currentTarget.dataset.id)); if (!item) return;
    const action = event.currentTarget.dataset.announcementAction;
    if (action === "edit") return openAnnouncementEditor(item);
    if (action === "read") return openAnnouncementReadStatus(item, 1, "");
    if (!confirm("确认删除这条公告？")) return;
    try { await api(`/api/admin/announcements/${item.id}`, { method: "DELETE" }); toast("公告已删除"); await renderRoute(); }
    catch (error) { toast(error.message, true); }
  }

  function openAnnouncementEditor(item = null) {
    editingTargeting = JSON.parse(JSON.stringify(item?.targeting || { any_of: [] }));
    openModal(item ? "编辑公告" : "创建公告", `<form id="announcement-form">
      <div class="field"><label for="announcement-title">标题</label><input id="announcement-title" name="title" value="${escapeHtml(item?.title || "")}" maxlength="160" required autofocus></div>
      <div class="field"><label for="announcement-content">内容</label><textarea id="announcement-content" name="content" required>${escapeHtml(item?.content || "")}</textarea></div>
      <div class="form-grid"><div class="field"><label for="announcement-status">状态</label><select id="announcement-status" name="status">${[["draft", "草稿"], ["active", "发布"], ["archived", "归档"]].map(([value, label]) => `<option value="${value}" ${(item?.status || "draft") === value ? "selected" : ""}>${label}</option>`).join("")}</select></div><div class="field"><label for="announcement-notify">通知模式</label><select id="announcement-notify" name="notify_mode"><option value="silent" ${(item?.notify_mode || "silent") === "silent" ? "selected" : ""}>静默</option><option value="popup" ${item?.notify_mode === "popup" ? "selected" : ""}>重要弹窗</option></select></div></div>
      <div class="form-grid"><div class="field"><label for="announcement-start">开始展示</label><input id="announcement-start" name="starts_at" type="datetime-local" value="${escapeHtml(toDateTimeLocal(item?.starts_at))}"></div><div class="field"><label for="announcement-end">结束展示</label><input id="announcement-end" name="ends_at" type="datetime-local" value="${escapeHtml(toDateTimeLocal(item?.ends_at))}"></div></div>
      <div id="announcement-targeting"></div><p class="form-error" id="announcement-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-announcement">保存</button>');
    modal.classList.add("user-detail-modal"); renderTargetingEditor();
    modal.querySelector("#save-announcement")?.addEventListener("click", () => saveAnnouncement(item?.id));
  }

  function defaultCondition() { return { type: "balance", operator: "gte", value: 0 }; }

  function renderTargetingEditor() {
    const root = modal.querySelector("#announcement-targeting"); if (!root) return;
    const groups = editingTargeting.any_of || [];
    root.innerHTML = `<div class="targeting-editor"><div class="targeting-heading"><div><strong>定向受众</strong><span>${groups.length ? "满足任意一组规则的用户" : "全部用户"}</span></div><div class="segmented"><label><input type="radio" name="target-mode" value="all" ${groups.length ? "" : "checked"}> 全部</label><label><input type="radio" name="target-mode" value="custom" ${groups.length ? "checked" : ""}> 自定义</label></div></div>${groups.length ? `<div class="target-groups">${groups.map((group, groupIndex) => `<div class="target-group"><header><strong>OR 规则组 ${groupIndex + 1}</strong><button type="button" class="button quiet small" data-target-remove-group="${groupIndex}">删除组</button></header>${(group.all_of || []).map((condition, conditionIndex) => targetCondition(condition, groupIndex, conditionIndex)).join("")}<button type="button" class="button secondary small" data-target-add-condition="${groupIndex}">添加 AND 条件</button></div>`).join("")}</div><button type="button" class="button secondary small" id="add-target-group">添加 OR 规则组</button>` : ""}</div>`;
    root.querySelectorAll('[name="target-mode"]').forEach(input => input.addEventListener("change", event => { editingTargeting = event.currentTarget.value === "all" ? { any_of: [] } : { any_of: [{ all_of: [defaultCondition()] }] }; renderTargetingEditor(); }));
    root.querySelector("#add-target-group")?.addEventListener("click", () => { if (editingTargeting.any_of.length < 50) editingTargeting.any_of.push({ all_of: [defaultCondition()] }); renderTargetingEditor(); });
    root.querySelectorAll("[data-target-remove-group]").forEach(button => button.addEventListener("click", () => { editingTargeting.any_of.splice(Number(button.dataset.targetRemoveGroup), 1); renderTargetingEditor(); }));
    root.querySelectorAll("[data-target-add-condition]").forEach(button => button.addEventListener("click", () => { const group = editingTargeting.any_of[Number(button.dataset.targetAddCondition)]; if (group.all_of.length < 50) group.all_of.push(defaultCondition()); renderTargetingEditor(); }));
    root.querySelectorAll("[data-target-remove-condition]").forEach(button => button.addEventListener("click", () => { const [group, condition] = button.dataset.targetRemoveCondition.split(":").map(Number); editingTargeting.any_of[group].all_of.splice(condition, 1); renderTargetingEditor(); }));
    root.querySelectorAll("[data-target-kind]").forEach(select => select.addEventListener("change", () => { const [group, condition] = select.dataset.targetKind.split(":").map(Number); editingTargeting.any_of[group].all_of[condition] = select.value === "subscription" ? { type: "subscription", operator: "in", group_ids: plans.length ? [plans[0].id] : [] } : defaultCondition(); renderTargetingEditor(); }));
    root.querySelectorAll("[data-target-operator]").forEach(select => select.addEventListener("change", () => { const [group, condition] = select.dataset.targetOperator.split(":").map(Number); editingTargeting.any_of[group].all_of[condition].operator = select.value; }));
    root.querySelectorAll("[data-target-value]").forEach(input => input.addEventListener("input", () => { const [group, condition] = input.dataset.targetValue.split(":").map(Number); editingTargeting.any_of[group].all_of[condition].value = Number(input.value); }));
    root.querySelectorAll("[data-target-plan]").forEach(input => input.addEventListener("change", () => { const [group, condition, plan] = input.dataset.targetPlan.split(":").map(Number); const target = editingTargeting.any_of[group].all_of[condition]; const values = new Set(target.group_ids || []); input.checked ? values.add(plan) : values.delete(plan); target.group_ids = [...values]; }));
  }

  function targetCondition(condition, groupIndex, conditionIndex) {
    const key = `${groupIndex}:${conditionIndex}`;
    const planIds = new Set(condition.group_ids || []);
    return `<div class="target-condition"><div class="field"><label>条件类型</label><select data-target-kind="${key}"><option value="balance" ${condition.type === "balance" ? "selected" : ""}>账户余额</option><option value="subscription" ${condition.type === "subscription" ? "selected" : ""}>有效套餐</option></select></div>${condition.type === "subscription" ? `<div class="target-plan-list">${plans.length ? plans.map(plan => `<label><input type="checkbox" data-target-plan="${key}:${plan.id}" ${planIds.has(plan.id) ? "checked" : ""}> ${escapeHtml(plan.name)}</label>`).join("") : '<span class="field-hint">尚未创建套餐</span>'}</div>` : `<div class="field"><label>比较</label><select data-target-operator="${key}">${[["gt", ">"], ["gte", "≥"], ["lt", "<"], ["lte", "≤"], ["eq", "="]].map(([value, label]) => `<option value="${value}" ${condition.operator === value ? "selected" : ""}>${label}</option>`).join("")}</select></div><div class="field"><label>余额（分）</label><input data-target-value="${key}" type="number" min="0" max="100000000000" value="${Number(condition.value || 0)}"></div>`}<button type="button" class="button quiet small" data-target-remove-condition="${key}">移除</button></div>`;
  }

  function validTargeting() {
    return (editingTargeting.any_of || []).every(group => (group.all_of || []).length > 0 && group.all_of.every(condition => condition.type !== "subscription" || (condition.group_ids || []).length > 0));
  }

  async function saveAnnouncement(id) {
    const form = modal.querySelector("#announcement-form"); if (!form.reportValidity()) return;
    const error = form.querySelector("#announcement-error");
    if (!validTargeting()) { error.textContent = "每个规则组都需要条件，套餐条件至少选择一个套餐"; return; }
    const values = Object.fromEntries(new FormData(form));
    values.starts_at = values.starts_at ? new Date(values.starts_at).toISOString() : null;
    values.ends_at = values.ends_at ? new Date(values.ends_at).toISOString() : null;
    values.targeting = editingTargeting;
    const button = modal.querySelector("#save-announcement"); button.disabled = true;
    try { await api(id ? `/api/admin/announcements/${id}` : "/api/admin/announcements", { method: id ? "PUT" : "POST", body: JSON.stringify(values) }); closeModal(); toast("公告已保存"); await renderRoute(); }
    catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
  }

  async function openAnnouncementReadStatus(item, page, search) {
    try {
      const query = new URLSearchParams({ page: String(page), page_size: "20", search });
      const result = await api(`/api/admin/announcements/${item.id}/read-status?${query}`);
      const table = result.data.length ? `<div class="table-wrap"><table><thead><tr><th>用户</th><th>余额</th><th>可见</th><th>阅读时间</th></tr></thead><tbody>${result.data.map(row => `<tr><td><span class="cell-main">${escapeHtml(row.email || row.username)}</span><span class="cell-sub">${escapeHtml(row.username)} · #${row.user_id}</span></td><td>${formatMoney(row.balance_cents)}</td><td>${row.eligible ? status("是") : status("否", "off")}</td><td>${row.read_at ? formatDate(row.read_at) : "未读"}</td></tr>`).join("")}</tbody></table></div>` : emptyState("没有匹配用户", "调整搜索条件后重试");
      openModal(`${item.title} · 已读状态`, `<form id="announcement-read-search" class="inline-search"><input name="search" type="search" value="${escapeHtml(search)}" placeholder="邮箱或用户名"><button class="button" type="submit">搜索</button></form>${table}${pager("announcement-read", result.meta)}`, '<button class="button" data-close-modal>关闭</button>');
      modal.classList.add("user-detail-modal");
      modal.querySelector("#announcement-read-search")?.addEventListener("submit", event => { event.preventDefault(); openAnnouncementReadStatus(item, 1, new FormData(event.currentTarget).get("search").trim()); });
      modal.querySelectorAll('[data-engagement-page="announcement-read"]').forEach(button => button.addEventListener("click", () => openAnnouncementReadStatus(item, Math.max(1, page + (button.dataset.direction === "next" ? 1 : -1)), search)));
    } catch (error) { toast(error.message, true); }
  }

  function pageAdminTable(items) {
    return `<div class="table-wrap"><table><thead><tr><th>页面</th><th>类型</th><th>公开</th><th>状态</th><th>排序</th><th></th></tr></thead><tbody>${items.map(item => `<tr><td><span class="cell-main">${escapeHtml(item.title)}</span><span class="cell-sub mono">${escapeHtml(item.slug)}</span></td><td>${item.kind === "legal" ? "法律文档" : "自定义"}</td><td>${item.public ? "公开" : "登录后"}</td><td>${item.enabled ? status("启用") : status("停用", "off")}</td><td>${item.sort_order}</td><td><div class="cell-actions"><button class="button quiet small" data-page-action="edit" data-id="${item.id}">编辑</button><button class="button quiet small" data-page-action="delete" data-id="${item.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function openContentPageEditor(item = null) {
    openModal(item ? "编辑内容页" : "创建内容页", `<form id="content-page-form"><div class="form-grid"><div class="field"><label for="page-title">标题</label><input id="page-title" name="title" value="${escapeHtml(item?.title || "")}" maxlength="160" required autofocus></div><div class="field"><label for="page-slug">Slug</label><input id="page-slug" name="slug" value="${escapeHtml(item?.slug || "")}" pattern="[a-z0-9-]+" maxlength="80" required></div></div><div class="form-grid"><div class="field"><label for="page-kind">类型</label><select id="page-kind" name="kind"><option value="custom" ${item?.kind !== "legal" ? "selected" : ""}>自定义</option><option value="legal" ${item?.kind === "legal" ? "selected" : ""}>法律文档</option></select></div><div class="field"><label for="page-sort">排序</label><input id="page-sort" name="sort_order" type="number" min="-10000" max="10000" value="${Number(item?.sort_order || 0)}"></div></div><div class="check-row"><label><input id="page-public" type="checkbox" ${item?.public ? "checked" : ""}> 允许未登录访问</label><label><input id="page-enabled" type="checkbox" ${item == null || item.enabled ? "checked" : ""}> 启用页面</label></div><div class="field"><label for="page-content">内容</label><textarea id="page-content" name="content" class="page-editor" placeholder="Markdown；自定义页面只填写 HTTP(S) URL 时使用 iframe">${escapeHtml(item?.content || "")}</textarea></div><p class="form-error" id="page-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-content-page">保存</button>');
    modal.querySelector("#save-content-page")?.addEventListener("click", () => saveContentPage(item?.id));
  }

  async function saveContentPage(id) {
    const form = modal.querySelector("#content-page-form"); if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form)); values.sort_order = Number(values.sort_order);
    values.public = form.querySelector("#page-public").checked; values.enabled = form.querySelector("#page-enabled").checked;
    const button = modal.querySelector("#save-content-page"); button.disabled = true;
    try { await api(id ? `/api/admin/pages/${id}` : "/api/admin/pages", { method: id ? "PUT" : "POST", body: JSON.stringify(values) }); closeModal(); toast("内容页已保存"); await renderRoute(); }
    catch (error) { form.querySelector("#page-error").textContent = error.message; button.disabled = false; }
  }

  async function handlePageAction(event) {
    const item = contentPages.find(row => row.id === Number(event.currentTarget.dataset.id)); if (!item) return;
    if (event.currentTarget.dataset.pageAction === "edit") return openContentPageEditor(item);
    if (!confirm("确认删除这个内容页？")) return;
    try { await api(`/api/admin/pages/${item.id}`, { method: "DELETE" }); toast("内容页已删除"); await renderRoute(); }
    catch (error) { toast(error.message, true); }
  }

  async function maybeShowPopup(force = false) {
    if (popupShowing || modal.open || currentRouteName() === "announcements") return;
    const now = Date.now(); if (!force && now - popupFetchedAt < 20 * 60 * 1000) return;
    popupFetchedAt = now;
    try {
      const result = await api("/api/user/announcements?unread_only=true");
      result.data.filter(item => item.notify_mode === "popup" && !shownPopups.has(item.id)).forEach(item => { if (!popupQueue.some(queued => queued.id === item.id)) popupQueue.push(item); });
      showNextPopup();
    } catch (_) { popupFetchedAt = 0; }
  }

  function showNextPopup() {
    if (popupShowing || modal.open || !popupQueue.length) return;
    const item = popupQueue.shift(); popupShowing = true; shownPopups.add(item.id);
    sessionStorage.setItem(popupStorageKey, JSON.stringify([...shownPopups]));
    openModal(item.title, `<div class="announcement-popup"><span>${formatDate(item.created_at)}</span><div class="markdown-body">${item.rendered_html || escapeHtml(item.content)}</div></div>`, '<button class="button" id="dismiss-announcement-popup">我知道了</button>');
    let handled = false;
    const dismiss = async () => {
      if (handled) return; handled = true; popupShowing = false;
      try { await api(`/api/user/announcements/${item.id}/read`, { method: "POST", body: "{}" }); } catch (_) {}
      setTimeout(showNextPopup, 300);
    };
    modal.addEventListener("close", dismiss, { once: true });
    modal.querySelector("#dismiss-announcement-popup")?.addEventListener("click", closeModal);
  }

  function reset() {
    popupQueue = []; popupShowing = false; popupFetchedAt = 0; shownPopups = new Set();
    sessionStorage.removeItem(popupStorageKey);
  }

  return { renderAnnouncements, renderContentAdmin, markAnnouncementRead, maybeShowPopup, reset };
})();
