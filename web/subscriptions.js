"use strict";

(() => {
  let plans = [];
  let subscriptions = [];
  let groups = [];

  function planMoney(cents, currency = "CNY") {
    try { return new Intl.NumberFormat("zh-CN", { style: "currency", currency: currency || "CNY" }).format((Number(cents) || 0) / 100); }
    catch (_) { return `${currency || "CNY"} ${((Number(cents) || 0) / 100).toFixed(2)}`; }
  }

  async function renderUser(page) {
    const [planResult, subscriptionResult, profile, orders] = await Promise.all([
      api("/api/user/plans"),
      api("/api/user/subscriptions"),
      api("/api/user/profile"),
      api("/api/user/orders"),
    ]);
    plans = planResult.data;
    subscriptions = subscriptionResult.data;
    const active = subscriptions.filter(item => item.status === "active");
    page.innerHTML = `
      ${pageHeader("我的订阅", `${active.length} 个有效订阅 · 余额 ${formatMoney(profile.data.balance_cents)}`)}
      ${active.length ? `<div class="subscription-list">${active.map(subscriptionHero).join("")}</div>` : emptyState("暂无有效订阅", "管理员分配套餐或购买套餐后会显示使用进度")}
      <section class="section"><div class="section-title"><h2>可用套餐</h2></div>${plans.length ? `<div class="plan-grid">${plans.map(plan => planCard(plan, profile.data.balance_cents)).join("")}</div>` : emptyState("暂无可用套餐", "管理员尚未发布套餐")}</section>
      <section class="section"><div class="section-title"><h2>订阅历史</h2></div>${subscriptions.length ? subscriptionTable(subscriptions, false) : emptyState("暂无订阅记录", "")}</section>
      <section class="section"><div class="section-title"><h2>订单历史</h2></div>${orders.data.length ? orderTable(orders.data, false) : emptyState("暂无订单", "余额购买套餐后会显示记录")}</section>`;
    page.querySelectorAll("[data-purchase-plan]").forEach(button => button.addEventListener("click", purchasePlan));
    page.querySelectorAll("[data-auto-renew]").forEach(input => input.addEventListener("change", updateAutoRenew));
    page.querySelectorAll("[data-renew-now]").forEach(button => button.addEventListener("click", retryRenewal));
  }

  function subscriptionHero(item) {
    const percent = item.token_limit ? Math.min(100, Math.round((item.used_tokens / item.token_limit) * 100)) : 0;
    return `<section class="subscription-hero"><div><span>${escapeHtml(item.group_name || "全局订阅")}</span><h2>${escapeHtml(item.plan_name)}</h2><p>${formatDate(item.starts_at)} - ${formatDate(item.ends_at)}</p><label class="toggle-line"><input type="checkbox" data-auto-renew="${item.id}" ${item.auto_renew ? "checked" : ""} ${item.renewal_price_cents > 0 ? "" : "disabled"}> 余额自动续订 · ${formatMoney(item.renewal_price_cents)}</label><div class="inline-field">${item.renewal_status === "insufficient_balance" ? `<button class="button secondary small" data-renew-now="${item.id}">充值后重试</button>` : ""}</div></div><div class="quota-progress"><div><span>该分组 Token 用量</span><strong>${item.token_limit ? `${formatNumber(item.used_tokens)} / ${formatNumber(item.token_limit)}` : `${formatNumber(item.used_tokens)} / 无限`}</strong></div><progress max="100" value="${percent}">${percent}%</progress><small>续订：${renewalLabel(item)}</small></div></section>`;
  }

  function renewalLabel(item) { return ({disabled:"关闭",scheduled:"等待到期",succeeded:"最近成功",insufficient_balance:"余额不足",plan_unavailable:"套餐不可用",error:"失败"})[item.renewal_status] || "关闭"; }
  async function updateAutoRenew(event) {
    event.currentTarget.disabled = true;
    try { await api(`/api/user/subscriptions/${event.currentTarget.dataset.autoRenew}/auto-renew`, { method:"PUT", body:JSON.stringify({enabled:event.currentTarget.checked}) }); toast("自动续订设置已更新"); await renderRoute(); }
    catch (error) { toast(error.message, true); event.currentTarget.checked = !event.currentTarget.checked; event.currentTarget.disabled = false; }
  }
  async function retryRenewal(event) {
    event.currentTarget.disabled = true;
    try { const result = await api(`/api/user/subscriptions/${event.currentTarget.dataset.renewNow}/renew`, { method:"POST", body:"{}" }); toast(result.data.renewed ? "续订成功" : "续订尚未完成"); await renderRoute(); }
    catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
  }

  function planCard(plan, balanceCents) {
    const features = Array.isArray(plan.features) ? plan.features : [];
    const price = plan.price_cents ? `<small>${plan.original_price_cents > plan.price_cents ? `<s>${planMoney(plan.original_price_cents, plan.currency)}</s> ` : ""}${planMoney(plan.price_cents, plan.currency)}</small>` : "<small>免费 / 手动分配</small>";
    return `<article class="plan-card"><header><span>${plan.duration_days} 天</span><h2>${escapeHtml(plan.name)}</h2></header>${plan.product_name ? `<span class="type-pill">${escapeHtml(plan.product_name)}</span>` : ""}<p>${escapeHtml(plan.description || "")}</p><span class="type-pill">${escapeHtml(plan.group_name || "全局")}</span><strong>${plan.token_limit ? `${formatNumber(plan.token_limit)} Token` : "无限 Token"}</strong>${features.length ? `<ul>${features.map(feature => `<li>${escapeHtml(feature)}</li>`).join("")}</ul>` : ""}${price}${plan.price_cents ? `<div class="plan-actions"><button class="button small" data-purchase-plan="${plan.id}" data-plan-name="${escapeHtml(plan.name)}" data-group-name="${escapeHtml(plan.group_name || "全局")}" data-price="${plan.price_cents}" ${balanceCents >= plan.price_cents ? "" : "disabled"}>余额购买</button></div>` : ""}</article>`;
  }

  function purchasePlan(event) {
    const { purchasePlan: planId, planName, groupName, price } = event.currentTarget.dataset;
    openModal("确认购买", `<p>使用 <strong>${formatMoney(price)}</strong> 购买 <strong>${escapeHtml(planName)}</strong>。</p><p class="field-hint">将替换“${escapeHtml(groupName)}”下的现有有效订阅，其他分组不受影响。</p><p class="form-error" id="purchase-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="confirm-purchase">确认购买</button>');
    modal.querySelector("#confirm-purchase").addEventListener("click", () => executePurchase(planId));
  }

  async function executePurchase(planId) {
    const button = modal.querySelector("#confirm-purchase");
    button.disabled = true;
    try {
      const result = await api("/api/user/purchase", { method: "POST", body: JSON.stringify({ plan_id: Number(planId) }) });
      closeModal();
      toast(`已购买 ${result.data.plan_name}`);
      await renderRoute();
    } catch (error) {
      modal.querySelector("#purchase-error").textContent = error.message;
      button.disabled = false;
    }
  }

  async function renderAdmin(page) {
    const [planResult, subscriptionResult, users, groupResult] = await Promise.all([
      api("/api/admin/plans"),
      api("/api/admin/subscriptions"),
      api("/api/admin/users"),
      api("/api/admin/groups"),
    ]);
    plans = planResult.data;
    subscriptions = subscriptionResult.data;
    groups = groupResult.data.filter(group => group.subscription_type === "subscription");
    page.innerHTML = `${pageHeader("套餐管理", `${plans.length} 个套餐`, `<button class="button secondary" id="assign-subscription" ${plans.some(plan => plan.enabled) ? "" : "disabled"}>分配订阅</button><button class="button" id="add-plan">创建套餐</button>`)}
      ${plans.length ? planTable(plans) : emptyState("暂无套餐", "创建套餐并绑定订阅分组", "创建套餐", "empty-add-plan")}
      <section class="section"><div class="section-title"><h2>订阅记录</h2></div>${subscriptions.length ? subscriptionTable(subscriptions, true) : emptyState("暂无订阅", "为用户分配套餐后会显示记录")}</section>`;
    page.querySelector("#add-plan")?.addEventListener("click", () => openPlanModal());
    page.querySelector("#empty-add-plan")?.addEventListener("click", () => openPlanModal());
    page.querySelector("#assign-subscription")?.addEventListener("click", () => openAssignModal(users.data));
    page.querySelectorAll("[data-plan-action]").forEach(button => button.addEventListener("click", handlePlanAction));
    page.querySelectorAll("[data-subscription-action]").forEach(button => button.addEventListener("click", cancelSubscription));
  }

  function planTable(items) {
    return `<div class="table-wrap"><table><thead><tr><th>套餐</th><th>订阅分组</th><th>Token</th><th>天数</th><th>价格</th><th>状态</th><th></th></tr></thead><tbody>${items.map(plan => `<tr><td><span class="cell-main">${escapeHtml(plan.name)}</span><span class="cell-sub">${escapeHtml(plan.product_name || plan.description || "")}</span></td><td>${escapeHtml(plan.group_name || "全局兼容套餐")}</td><td>${plan.token_limit ? formatNumber(plan.token_limit) : "无限"}</td><td>${plan.duration_days}</td><td><span class="cell-main">${planMoney(plan.price_cents, plan.currency)}</span>${plan.original_price_cents > plan.price_cents ? `<span class="cell-sub"><s>${planMoney(plan.original_price_cents, plan.currency)}</s></span>` : ""}</td><td>${plan.enabled ? status("启用") : status("停用", "off")}</td><td><div class="cell-actions"><button class="button quiet small" data-plan-action="edit" data-id="${plan.id}">编辑</button><button class="button quiet small" data-plan-action="delete" data-id="${plan.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function subscriptionTable(items, admin) {
    return `<div class="table-wrap"><table><thead><tr>${admin ? "<th>用户</th>" : ""}<th>套餐</th><th>分组</th><th>状态</th><th>续订</th><th>Token</th><th>有效期</th>${admin ? "<th></th>" : ""}</tr></thead><tbody>${items.map(item => `<tr>${admin ? `<td class="mono">${escapeHtml(item.username)}</td>` : ""}<td class="cell-main">${escapeHtml(item.plan_name)}</td><td>${escapeHtml(item.group_name || "全局")}</td><td>${item.status === "active" ? status("有效") : item.status === "expired" ? status("已过期", "warn") : status("已取消", "off")}</td><td>${renewalLabel(item)}</td><td>${formatNumber(item.used_tokens)} / ${item.token_limit ? formatNumber(item.token_limit) : "无限"}</td><td>${formatDate(item.starts_at)} - ${formatDate(item.ends_at)}</td>${admin ? `<td>${item.status === "active" ? `<button class="button quiet small" data-subscription-action="cancel" data-id="${item.id}">取消</button>` : ""}</td>` : ""}</tr>`).join("")}</tbody></table></div>`;
  }

  function openPlanModal(plan = null) {
    const options = groups.map(group => `<option value="${group.id}" ${String(group.id) === String(plan?.group_id) ? "selected" : ""}>${escapeHtml(group.name)}</option>`).join("");
    openModal(plan ? "编辑套餐" : "创建套餐", `<form id="plan-form">
      <div class="field"><label for="plan-name">名称</label><input id="plan-name" name="name" value="${escapeHtml(plan?.name || "")}" maxlength="80" required autofocus></div>
      <div class="field"><label for="plan-description">说明</label><textarea id="plan-description" name="description" class="compact-textarea">${escapeHtml(plan?.description || "")}</textarea></div>
      <div class="field"><label for="plan-product-name">商品展示名</label><input id="plan-product-name" name="product_name" value="${escapeHtml(plan?.product_name || "")}" maxlength="100"></div>
      <div class="field"><label for="plan-features">功能特性</label><textarea id="plan-features" name="features" class="compact-textarea" placeholder="每行一个特性">${escapeHtml((plan?.features || []).join("\n"))}</textarea></div>
      <div class="field"><label for="plan-group">订阅分组</label><select id="plan-group" name="group_id"><option value="0" ${plan?.group_id ? "" : "selected"}>全局兼容套餐</option>${options}</select><span class="field-hint">绑定后，订阅仅授权并统计该分组；全局模式仅用于兼容旧数据。</span></div>
      <div class="form-grid"><div class="field"><label for="plan-token-limit">Token 上限</label><input id="plan-token-limit" name="token_limit" type="number" min="0" value="${Number(plan?.token_limit || 0)}" required><span class="field-hint">0 表示无限</span></div><div class="field"><label for="plan-days">有效天数</label><input id="plan-days" name="duration_days" type="number" min="1" max="3650" value="${Number(plan?.duration_days || 30)}" required></div></div>
      <div class="form-grid"><div class="field"><label for="plan-price">价格（最小货币单位）</label><input id="plan-price" name="price_cents" type="number" min="0" value="${Number(plan?.price_cents || 0)}"></div><div class="field"><label for="plan-original-price">原价</label><input id="plan-original-price" name="original_price_cents" type="number" min="0" value="${Number(plan?.original_price_cents || 0)}"></div></div>
      <div class="form-grid"><div class="field"><label for="plan-currency">展示币种</label><input id="plan-currency" name="currency" value="${escapeHtml(plan?.currency || "CNY")}" minlength="3" maxlength="3" pattern="[A-Za-z]{3}" required><span class="field-hint">ISO 三字母展示标签；实际在线扣款使用支付渠道币种</span></div><div class="field"><label for="plan-sort">排序</label><input id="plan-sort" name="sort_order" type="number" min="-10000" max="10000" value="${Number(plan?.sort_order || 0)}"></div></div>
      <label class="toggle-line"><input id="plan-enabled" type="checkbox" ${plan == null || plan.enabled ? "checked" : ""}> 启用套餐</label><p class="form-error" id="plan-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-plan">保存</button>`);
    modal.querySelector("#save-plan").addEventListener("click", () => savePlan(plan?.id));
  }

  async function savePlan(id) {
    const form = modal.querySelector("#plan-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    for (const key of ["token_limit", "duration_days", "price_cents", "original_price_cents", "sort_order", "group_id"]) values[key] = Number(values[key]);
    values.currency = values.currency.trim().toUpperCase();
    values.features = values.features.split("\n").map(value => value.trim()).filter(Boolean);
    values.enabled = form.querySelector("#plan-enabled").checked;
    const button = modal.querySelector("#save-plan");
    button.disabled = true;
    try {
      await api(id ? `/api/admin/plans/${id}` : "/api/admin/plans", { method: id ? "PUT" : "POST", body: JSON.stringify(values) });
      closeModal(); toast("套餐已保存"); await renderRoute();
    } catch (error) { modal.querySelector("#plan-error").textContent = error.message; button.disabled = false; }
  }

  function handlePlanAction(event) {
    const plan = plans.find(item => String(item.id) === event.currentTarget.dataset.id);
    if (!plan) return;
    if (event.currentTarget.dataset.planAction === "edit") return openPlanModal(plan);
    openModal("删除套餐", `<p>确认删除 <strong>${escapeHtml(plan.name)}</strong>？有订阅、兑换码或订单历史时会拒绝删除。</p><p class="form-error" id="plan-delete-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-delete-plan">删除</button>');
    modal.querySelector("#confirm-delete-plan").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try { await api(`/api/admin/plans/${plan.id}`, { method: "DELETE" }); closeModal(); toast("套餐已删除"); await renderRoute(); }
      catch (error) { modal.querySelector("#plan-delete-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  function openAssignModal(users) {
    const enabledPlans = plans.filter(plan => plan.enabled);
    openModal("分配订阅", `<form id="assign-subscription-form"><div class="field"><label for="subscription-user">用户</label><select id="subscription-user" name="user_id">${users.filter(user => user.enabled && user.role === "user").map(user => `<option value="${user.id}">${escapeHtml(user.display_name)} (${escapeHtml(user.username)})</option>`).join("")}</select></div><div class="field"><label for="subscription-plan">套餐</label><select id="subscription-plan" name="plan_id">${enabledPlans.map(plan => `<option value="${plan.id}">${escapeHtml(plan.name)} · ${escapeHtml(plan.group_name || "全局")}</option>`).join("")}</select></div><div class="form-grid"><div class="field"><label for="subscription-tokens">自定义 Token 上限</label><input id="subscription-tokens" name="token_limit" type="number" min="0" placeholder="留空使用套餐值"></div><div class="field"><label for="subscription-days">自定义天数</label><input id="subscription-days" name="duration_days" type="number" min="1" max="3650" placeholder="留空使用套餐值"></div></div><p class="form-error" id="subscription-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-subscription">分配</button>');
    modal.querySelector("#save-subscription").addEventListener("click", saveSubscription);
  }

  async function saveSubscription() {
    const form = modal.querySelector("#assign-subscription-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    values.user_id = Number(values.user_id); values.plan_id = Number(values.plan_id);
    values.token_limit = values.token_limit === "" ? null : Number(values.token_limit);
    values.duration_days = values.duration_days === "" ? null : Number(values.duration_days);
    const button = modal.querySelector("#save-subscription");
    button.disabled = true;
    try { await api("/api/admin/subscriptions", { method: "POST", body: JSON.stringify(values) }); closeModal(); toast("订阅已分配"); await renderRoute(); }
    catch (error) { modal.querySelector("#subscription-error").textContent = error.message; button.disabled = false; }
  }

  function cancelSubscription(event) {
    const item = subscriptions.find(value => String(value.id) === event.currentTarget.dataset.id);
    if (!item) return;
    openModal("取消订阅", `<p>确认取消 <strong>${escapeHtml(item.username)} · ${escapeHtml(item.plan_name)}</strong>？该分组的 Key 将立即失去访问权限。</p><p class="form-error" id="subscription-cancel-error"></p>`, '<button class="button secondary" data-close-modal>返回</button><button class="button danger" id="confirm-cancel-subscription">取消订阅</button>');
    modal.querySelector("#confirm-cancel-subscription").addEventListener("click", async event => {
      event.currentTarget.disabled = true;
      try { await api(`/api/admin/subscriptions/${item.id}`, { method: "PUT", body: JSON.stringify({ status: "cancelled" }) }); closeModal(); toast("订阅已取消"); await renderRoute(); }
      catch (error) { modal.querySelector("#subscription-cancel-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  window.Sub2MiniSubscriptions = { renderUser, renderAdmin };
})();
