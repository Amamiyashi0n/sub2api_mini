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

  return { attach };
})();
