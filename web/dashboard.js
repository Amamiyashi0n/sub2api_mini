"use strict";

window.Sub2MiniDashboard = (() => {
  let selectedRange = "7d";

  function rangeLabel(value) {
    return ({ "24h": "24 小时", "7d": "7 天", "30d": "30 天", "90d": "90 天" })[value] || value;
  }

  function rangeControl() {
    return `<div class="segmented dashboard-range" role="group" aria-label="统计范围">${["24h", "7d", "30d", "90d"].map(value => `<button class="${value === selectedRange ? "active" : ""}" data-dashboard-range="${value}" type="button">${rangeLabel(value)}</button>`).join("")}</div><button class="button secondary" id="refresh-overview">刷新</button>`;
  }

  function trendChart(rows) {
    if (!rows.length) return emptyState("暂无趋势数据", "所选时间范围内没有请求");
    const max = Math.max(...rows.map(row => Number(row.requests) || 0), 1);
    return `<div class="dashboard-trend" aria-label="请求趋势">${rows.map(row => {
      const success = Number(row.successful_requests) || 0;
      const failed = Number(row.failed_requests) || 0;
      const height = Math.max(5, (Number(row.requests) || 0) / max * 100);
      const successShare = row.requests ? success / row.requests * 100 : 100;
      return `<div class="dashboard-day" title="${escapeHtml(row.date)} · ${formatNumber(row.requests)} 请求 · ${formatNumber(row.tokens)} Token"><div class="dashboard-column" style="height:${height}%"><i style="height:${successShare}%"></i><b style="height:${100-successShare}%"></b></div><strong>${formatNumber(row.requests)}</strong><span>${escapeHtml(row.date.slice(5))}</span><small>${failed ? `${failed} 错误` : formatUsdMicros(row.cost_microusd)}</small></div>`;
    }).join("")}</div>`;
  }

  function bars(rows, label = "name") {
    if (!rows.length) return emptyState("暂无分布数据", "所选时间范围内没有请求");
    const max = Math.max(...rows.map(row => Number(row.requests) || 0), 1);
    return `<div class="dashboard-bars">${rows.map(row => `<div><span title="${escapeHtml(row[label])}">${escapeHtml(row[label])}</span><div><i style="width:${Math.max(2, row.requests/max*100)}%"></i></div><strong>${formatNumber(row.requests)}</strong><small>${formatUsdMicros(row.cost_microusd)}</small></div>`).join("")}</div>`;
  }

  function subscriptionPanel(subscription) {
    if (!subscription) return `<section class="dashboard-subscription"><div><span>当前套餐</span><strong>未订阅</strong><small>可使用余额购买套餐或兑换订阅</small></div><div class="dashboard-actions"><a class="button" href="#/subscriptions">选择套餐</a><a class="button secondary" href="#/redeem">兑换</a></div></section>`;
    const percent = subscription.token_limit ? Math.min(100, subscription.used_tokens / subscription.token_limit * 100) : 0;
    return `<section class="dashboard-subscription"><div><span>当前套餐</span><strong>${escapeHtml(subscription.plan_name)}</strong><small>有效至 ${formatDate(subscription.ends_at)}</small></div><div class="subscription-progress"><span>${subscription.token_limit ? `${formatNumber(subscription.used_tokens)} / ${formatNumber(subscription.token_limit)} Token` : "无限 Token"}</span><div><i style="width:${percent}%"></i></div></div><a class="button secondary" href="#/subscriptions">订阅详情</a></section>`;
  }

  function rankingTable(rows, label) {
    if (!rows.length) return emptyState("暂无排行数据", "所选范围内没有请求");
    return `<div class="table-wrap"><table><thead><tr><th>${label}</th><th>请求</th><th>Token</th><th>成本</th></tr></thead><tbody>${rows.map(row => `<tr><td>${escapeHtml(row.username || row.name)}</td><td>${formatNumber(row.requests)}</td><td>${formatNumber(row.tokens)}</td><td>${formatUsdMicros(row.cost_microusd)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function adminMetrics(data) {
    const period = data.period;
    return `${metric("用户", `${data.active_users}/${data.users}`, "good")}${metric("新增用户", data.new_users)}${metric("可用账号", `${data.active_accounts}/${data.accounts}`, "good")}${metric("有效 Key", `${data.active_keys}/${data.keys}`, "good")}${metric("请求", formatNumber(period.requests))}${metric("成功率", `${Number(period.success_rate).toFixed(2)}%`, period.failed_requests ? "warn" : "good")}${metric("Token", formatNumber(period.total_tokens))}${metric("API 成本", formatUsdMicros(period.cost_microusd))}${metric("订单收入", formatMoney(data.period_revenue_cents), "good")}${metric("有效订阅", data.active_subscriptions)}`;
  }

  function userMetrics(data) {
    const period = data.period;
    return `${metric("可用余额", formatMoney(data.balance_cents), "good")}${metric("有效 Key", `${data.active_keys}/${data.total_api_keys}`, "good")}${metric("请求", formatNumber(period.requests))}${metric("成功率", `${Number(period.success_rate).toFixed(2)}%`, period.failed_requests ? "warn" : "good")}${metric("总 Token", formatNumber(period.total_tokens))}${metric("缓存 Token", formatNumber(period.cached_input_tokens))}${metric("推理 Token", formatNumber(period.reasoning_tokens))}${metric("成本", formatUsdMicros(period.cost_microusd))}${metric("RPM / TPM", `${formatNumber(data.rpm)} / ${formatNumber(data.tpm)}`)}${metric("平均耗时", `${formatNumber(period.average_duration_ms)} ms`)}`;
  }

  async function render(page) {
    const base = roleApiBase();
    const [summary, usage, announcements] = await Promise.all([
      api(`${base}/dashboard?range=${encodeURIComponent(selectedRange)}`),
      api(`${base}/usage?page_size=8`),
      api("/api/user/announcements"),
    ]);
    const data = summary.data;
    const metrics = state.role === "admin" ? adminMetrics(data) : userMetrics(data);
    page.innerHTML = `${pageHeader("概览", `近 ${rangeLabel(selectedRange)} · ${Number(data.period.success_rate).toFixed(2)}% 成功率`, rangeControl())}
      <section class="metric-grid dashboard-metrics">${metrics}</section>
      ${state.role === "admin" ? `<section class="dashboard-finance"><div><span>累计已支付</span><strong>${formatMoney(data.revenue_cents)}</strong><small>${formatNumber(data.paid_orders)} 个订单</small></div><div><span>累计退款</span><strong>${formatMoney(data.refunded_cents)}</strong><small>${formatNumber(data.refunded_orders)} 个订单</small></div><div><span>累计 API 请求</span><strong>${formatNumber(data.total.requests)}</strong><small>${formatUsdMicros(data.total.cost_microusd)} 计量成本</small></div></section>` : subscriptionPanel(data.subscription)}
      <section class="dashboard-chart-grid">
        <div class="dashboard-panel dashboard-trend-panel"><div class="section-title"><h2>请求趋势</h2><a href="#/usage">用量明细</a></div>${trendChart(data.trend || [])}</div>
        <div class="dashboard-panel"><div class="section-title"><h2>模型分布</h2></div>${bars(data.models || [], "model")}</div>
        <div class="dashboard-panel"><div class="section-title"><h2>端点分布</h2></div>${bars(data.endpoints || [])}</div>
      </section>
      ${state.role === "admin" ? `<section class="dashboard-ranking-grid"><div><div class="section-title"><h2>用户消费排行</h2><a href="#/users">用户管理</a></div>${rankingTable(data.top_users || [], "用户")}</div><div><div class="section-title"><h2>分组请求</h2><a href="#/groups">路由分组</a></div>${rankingTable(data.groups || [], "分组")}</div></section>` : `<section class="dashboard-quick-actions"><a href="#/keys"><strong>API Key</strong><span>创建与管理客户端密钥</span></a><a href="#/subscriptions"><strong>套餐</strong><span>查看额度与订阅历史</span></a><a href="#/redeem"><strong>兑换</strong><span>激活订阅兑换码</span></a><a href="#/usage"><strong>用量</strong><span>筛选和导出请求记录</span></a></section>`}
      ${announcements.data.length ? `<section class="section"><div class="section-title"><h2>近期公告</h2><a href="#/announcements">查看全部</a></div>${announcementCards(announcements.data.slice(0, 3), true)}</section>` : ""}
      <section class="section"><div class="section-title"><h2>最近请求</h2><a href="#/usage">查看全部</a></div>${usageTable(usage.data)}</section>`;
    page.querySelector("#refresh-overview").addEventListener("click", renderRoute);
    page.querySelectorAll("[data-dashboard-range]").forEach(button => button.addEventListener("click", () => { selectedRange = button.dataset.dashboardRange; renderRoute(); }));
    page.querySelectorAll("[data-read-announcement]").forEach(button => button.addEventListener("click", markAnnouncementRead));
  }

  return { render };
})();
