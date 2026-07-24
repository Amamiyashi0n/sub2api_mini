"use strict";

(() => {
  let selectedRange = "24h";
  let refreshTimer = null;
  let liveSource = null;
  let settings = null;
  let rules = [];
  let requestPage = 1;
  let requestFilters = { kind: "all", model: "", request_id: "", min_duration_ms: "" };

  async function render(page) {
    stopRefresh();
    stopLiveMetrics();
    const [overviewResult, requestResult, ruleResult, eventResult, settingsResult, logResult, reportResult] = await Promise.all([
      api(`/api/admin/ops/overview?range=${encodeURIComponent(selectedRange)}`),
      fetchRequests(),
      api("/api/admin/ops/alert-rules"),
      api("/api/admin/ops/alert-events?limit=50"),
      api("/api/admin/ops/settings"),
      api("/api/admin/ops/system-logs?limit=50"),
      api("/api/admin/ops/reports"),
    ]);
    const data = overviewResult.data;
    settings = settingsResult.data;
    rules = ruleResult.data;
    const summary = data.summary;
    page.innerHTML = `
      ${pageHeader("运行运维", `${rangeLabel(selectedRange)} · ${data.preaggregated_trend ? "分钟汇总" : "原始明细"} · 生成于 ${formatDate(data.generated_at)}`, `<select id="ops-range" aria-label="统计范围">${["5m", "30m", "1h", "6h", "24h", "7d"].map(value => `<option value="${value}" ${value === selectedRange ? "selected" : ""}>${rangeLabel(value)}</option>`).join("")}</select><button class="button secondary" id="ops-report-run">发送报表</button><button class="button secondary" id="ops-evaluate">评估告警</button><button class="button secondary" id="ops-settings">设置</button><button class="button" id="ops-refresh">刷新</button>`)}
      <section class="metric-grid ops-metrics">${metric("健康分", `${data.health_score}/100`, data.health_score >= 90 ? "good" : "warn")}${metric("请求", formatNumber(summary.request_count))}${metric("成功率", `${Number(summary.success_rate).toFixed(2)}%`, summary.success_rate >= 99 ? "good" : "warn")}${metric("错误", formatNumber(summary.error_count), summary.error_count ? "warn" : "good")}${metric("峰值 QPS", formatRate(summary.qps.peak))}${metric("P95 延迟", `${formatNumber(summary.latency.p95_ms)} ms`, summary.latency.p95_ms > 5000 ? "warn" : "")}${metric("平均 TTFT", `${formatNumber(summary.telemetry.average_ttft_ms)} ms`)}${metric("切换率", `${Number(summary.telemetry.switch_rate).toFixed(2)}%`, summary.telemetry.account_switches ? "warn" : "good")}</section>
      ${liveStrip(summary, data.system)}
      ${systemStrip(data.system)}
      <section class="ops-grid"><div class="ops-panel ops-throughput"><div class="section-title"><h2>吞吐趋势</h2></div>${trendTable(data.trend)}</div><div class="ops-panel"><div class="section-title"><h2>延迟分布</h2></div>${barList(data.latency_histogram, "range", "count")}</div><div class="ops-panel"><div class="section-title"><h2>错误状态</h2></div>${data.errors.length ? barList(data.errors, "status_code", "count") : emptyState("暂无错误", "当前范围没有失败请求")}</div><div class="ops-panel"><div class="section-title"><h2>模型统计</h2></div>${modelTable(data.models)}</div></section>
      <section class="section"><div class="section-title"><h2>上游账号可用性</h2><a href="#/accounts">管理账号</a></div>${accountTable(data.accounts)}</section>
      <section class="section ops-request-section"><div class="section-title"><h2>请求与错误明细</h2></div>${requestFilter()}<div id="ops-request-list">${requestTable(requestResult)}</div></section>
      <section class="ops-grid ops-alert-grid"><div class="ops-panel"><div class="section-title"><h2>告警规则</h2><button class="button secondary small" id="add-alert-rule">添加规则</button></div>${alertRuleTable(rules)}</div><div class="ops-panel"><div class="section-title"><h2>告警事件</h2></div>${alertEventTable(eventResult.data)}</div></section>
      <section class="section"><div class="section-title"><h2>运维报表</h2><span class="field-hint">日报与周报按 UTC 调度</span></div>${reportTable(reportResult.data)}</section>
      <section class="section"><div class="section-title"><h2>系统日志</h2></div>${systemLogTable(logResult.data)}</section>`;
    bindPage(page);
    startLiveMetrics();
    scheduleRefresh();
  }

  function bindPage(page) {
    page.querySelector("#ops-range").addEventListener("change", event => { selectedRange = event.currentTarget.value; requestPage = 1; renderRoute(); });
    page.querySelector("#ops-refresh").addEventListener("click", renderRoute);
    page.querySelector("#ops-evaluate").addEventListener("click", evaluateAlerts);
    page.querySelector("#ops-report-run").addEventListener("click", runReport);
    page.querySelector("#ops-settings").addEventListener("click", openSettings);
    page.querySelector("#add-alert-rule").addEventListener("click", () => openAlertRule());
    page.querySelector("#ops-request-filter").addEventListener("submit", applyRequestFilter);
    bindRequestActions(page);
    page.querySelectorAll("[data-alert-action]").forEach(button => button.addEventListener("click", handleAlertAction));
    page.querySelectorAll("[data-event-resolve]").forEach(button => button.addEventListener("click", resolveEvent));
  }

  function liveStrip(summary, system) {
    return `<section class="ops-system-strip"><div><span>实时 QPS</span><strong id="ops-live-qps">${formatRate(summary.qps.current)}</strong></div><div><span>实时 TPS</span><strong id="ops-live-tps">${formatRate(summary.tps.current)}</strong></div><div><span>活跃网关请求</span><strong id="ops-live-active">${system.active_gateway_requests}</strong></div><div><span>近 1 分钟 TTFT</span><strong id="ops-live-ttft">${formatNumber(summary.telemetry.average_ttft_ms)} ms</strong></div><div><span>账号切换</span><strong id="ops-live-switches">${formatNumber(summary.telemetry.account_switches)}</strong></div><div><span>上游尝试</span><strong id="ops-live-attempts">${formatNumber(summary.telemetry.upstream_attempts)}</strong></div></section>`;
  }

  function systemStrip(system) {
    return `<section class="ops-system-strip"><div><span>进程 RSS</span><strong>${formatNumber(system.rss_kb)} KB</strong></div><div><span>cgroup 内存</span><strong>${formatBytes(system.cgroup_memory_bytes)}</strong></div><div><span>线程</span><strong>${system.threads}</strong></div><div><span>SQLite 连接</span><strong>${system.db_pool_size - system.db_idle_connections} / ${system.db_pool_size}</strong></div><div><span>数据库日志</span><strong>${settings.runtime_log.db_enabled ? "开启" : "关闭"}</strong></div><div><span>运行时间</span><strong>${formatDuration(system.uptime_seconds)}</strong></div></section>`;
  }

  function trendTable(rows) {
    if (!rows.length) return emptyState("暂无趋势", "完成网关请求后会显示吞吐变化");
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>请求</th><th>错误</th><th>QPS</th><th>TPS</th><th>成本</th></tr></thead><tbody>${rows.slice(-48).map(row => `<tr><td class="mono">${escapeHtml(row.bucket.slice(5))}</td><td>${formatNumber(row.requests)}</td><td>${row.errors ? status(row.errors, "warn") : "0"}</td><td>${formatRate(row.qps)}</td><td>${formatRate(row.tps)}</td><td>${formatMicrousd(row.cost_microusd)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function barList(rows, labelKey, valueKey) {
    const max = Math.max(...rows.map(row => Number(row[valueKey]) || 0), 1);
    return `<div class="ops-bars">${rows.map(row => `<div><span>${escapeHtml(String(row[labelKey]))}</span><div><i style="width:${Math.max(2, (Number(row[valueKey]) || 0) / max * 100)}%"></i></div><strong>${formatNumber(row[valueKey])}</strong></div>`).join("")}</div>`;
  }

  function modelTable(rows) {
    if (!rows.length) return emptyState("暂无模型", "当前范围没有模型调用");
    return `<div class="table-wrap"><table><thead><tr><th>模型</th><th>请求</th><th>Token</th><th>均延迟</th><th>成本</th></tr></thead><tbody>${rows.map(row => `<tr><td class="mono">${escapeHtml(row.model)}</td><td>${formatNumber(row.requests)}</td><td>${formatNumber(row.tokens)}</td><td>${formatNumber(row.average_duration_ms)} ms</td><td>${formatMicrousd(row.cost_microusd)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function accountTable(rows) {
    if (!rows.length) return emptyState("暂无上游账号", "添加账号后可查看可用性和错误");
    return `<div class="table-wrap"><table><thead><tr><th>账号</th><th>类型</th><th>状态</th><th>并发上限</th><th>请求</th><th>错误</th><th>最后使用</th></tr></thead><tbody>${rows.map(row => `<tr><td><span class="cell-main">${escapeHtml(row.name)}</span>${row.last_error ? `<span class="cell-sub">${escapeHtml(row.last_error)}</span>` : ""}</td><td>${row.kind === "oauth" ? "OAuth" : "API Key"}</td><td>${row.available ? status("可用") : row.cooldown_until ? status("冷却", "warn") : status("停用", "off")}</td><td>${row.concurrency}</td><td>${formatNumber(row.requests)}</td><td>${row.errors ? status(row.errors, "warn") : "0"}</td><td>${formatDate(row.last_used_at)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function requestFilter() {
    return `<form id="ops-request-filter" class="filter-bar ops-request-filter"><div class="field"><label for="ops-kind">结果</label><select id="ops-kind" name="kind"><option value="all" ${requestFilters.kind === "all" ? "selected" : ""}>全部</option><option value="success" ${requestFilters.kind === "success" ? "selected" : ""}>成功</option><option value="error" ${requestFilters.kind === "error" ? "selected" : ""}>错误</option></select></div><div class="field"><label for="ops-model">模型</label><input id="ops-model" name="model" value="${escapeHtml(requestFilters.model)}"></div><div class="field"><label for="ops-request-id">请求 ID</label><input id="ops-request-id" name="request_id" value="${escapeHtml(requestFilters.request_id)}"></div><div class="field"><label for="ops-duration">最小耗时 (ms)</label><input id="ops-duration" name="min_duration_ms" type="number" min="0" value="${escapeHtml(requestFilters.min_duration_ms)}"></div><button class="button secondary" type="submit">筛选</button></form>`;
  }

  async function fetchRequests() {
    const params = new URLSearchParams({ range: selectedRange, page: String(requestPage), page_size: "30", kind: requestFilters.kind });
    for (const [key, value] of Object.entries(requestFilters)) if (key !== "kind" && value !== "") params.set(key, value);
    return api(`/api/admin/ops/requests?${params}`);
  }

  function requestTable(result) {
    const rows = result.data;
    const meta = result.meta;
    if (!rows.length) return emptyState("暂无请求", "当前筛选条件没有记录");
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>请求 ID</th><th>用户 / Key</th><th>上游</th><th>端点 / 模型</th><th>状态</th><th>Token / 成本</th><th>延迟 / TTFT</th><th>尝试 / 切换</th><th></th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td><td class="mono">${escapeHtml(row.request_id.slice(0, 16))}</td><td><span class="cell-main">${escapeHtml(row.username || "system")}</span><span class="cell-sub">${escapeHtml(row.api_key_name || "-")}</span></td><td>${escapeHtml(row.account_name || "-")}</td><td><span class="cell-main">${escapeHtml(row.endpoint)}</span><span class="cell-sub mono">${escapeHtml(row.model || "-")}</span></td><td>${row.status_code < 400 ? status(row.status_code) : status(row.status_code, "error")}${row.error_summary ? `<span class="cell-sub">${escapeHtml(row.error_summary)}</span>` : ""}</td><td>${formatNumber(row.total_tokens)} / ${formatMicrousd(row.cost_microusd)}</td><td><span class="cell-main">${formatNumber(row.duration_ms)} ms</span><span class="cell-sub">TTFT ${row.ttft_ms == null ? "-" : `${formatNumber(row.ttft_ms)} ms`}</span></td><td>${formatNumber(row.upstream_attempts)} / ${formatNumber(row.account_switches)}</td><td><button class="button quiet small" data-request-detail="${row.id}">详情</button></td></tr>`).join("")}</tbody></table></div><div class="pagination"><button class="button quiet small" id="ops-request-prev" ${meta.page <= 1 ? "disabled" : ""}>上一页</button><span>第 ${meta.page} 页 · ${meta.total} 条</span><button class="button quiet small" id="ops-request-next" ${meta.page * meta.page_size >= meta.total ? "disabled" : ""}>下一页</button></div>`;
  }

  function bindRequestActions(page) {
    page.querySelectorAll("[data-request-detail]").forEach(button => button.addEventListener("click", openRequestDetail));
    page.querySelector("#ops-request-prev")?.addEventListener("click", () => changeRequestPage(-1));
    page.querySelector("#ops-request-next")?.addEventListener("click", () => changeRequestPage(1));
  }

  async function applyRequestFilter(event) {
    event.preventDefault();
    const values = Object.fromEntries(new FormData(event.currentTarget));
    requestFilters = { kind: values.kind, model: values.model.trim(), request_id: values.request_id.trim(), min_duration_ms: values.min_duration_ms };
    requestPage = 1;
    await refreshRequestList();
  }

  async function changeRequestPage(delta) { requestPage += delta; await refreshRequestList(); }
  async function refreshRequestList() {
    const container = document.querySelector("#ops-request-list");
    container.innerHTML = `<div class="boot-screen"><p>正在载入</p></div>`;
    try { container.innerHTML = requestTable(await fetchRequests()); bindRequestActions(document.querySelector("#page")); }
    catch (error) { container.innerHTML = emptyState("载入失败", error.message); }
  }

  async function openRequestDetail(event) {
    const result = await api(`/api/admin/ops/requests/${event.currentTarget.dataset.requestDetail}`);
    const row = result.data;
    openModal("请求详情", `<dl class="detail-list">${detail("请求 ID", row.request_id, true)}${detail("时间", formatDate(row.created_at))}${detail("用户", `${row.username || "system"} (#${row.user_id || "-"})`)}${detail("API Key", `${row.api_key_name || "-"} (#${row.api_key_id || "-"})`)}${detail("上游账号", `${row.account_name || "-"} (#${row.account_id || "-"})`)}${detail("端点", row.endpoint)}${detail("模型", row.model || "-")}${detail("状态", row.status_code)}${detail("Token", `${row.input_tokens || 0} + ${row.output_tokens || 0} = ${row.total_tokens || 0}`)}${detail("成本", formatMicrousd(row.cost_microusd))}${detail("总耗时", `${row.duration_ms} ms`)}${detail("首 Token", row.ttft_ms == null ? "-" : `${row.ttft_ms} ms`)}${detail("上游尝试", row.upstream_attempts)}${detail("账号切换", row.account_switches)}${detail("错误摘要", row.error_summary || "-")}</dl><p class="field-hint">运维日志不保存请求正文或模型输出。</p>`, `<button class="button" data-close-modal>关闭</button>`);
  }

  function detail(label, value, mono = false) { return `<div><dt>${escapeHtml(label)}</dt><dd class="${mono ? "mono" : ""}">${escapeHtml(String(value))}</dd></div>`; }

  function alertRuleTable(rows) {
    if (!rows.length) return emptyState("暂无规则", "添加规则后每分钟自动评估");
    return `<div class="table-wrap"><table><thead><tr><th>规则</th><th>条件</th><th>窗口</th><th>级别</th><th>状态</th><th></th></tr></thead><tbody>${rows.map(row => `<tr><td><span class="cell-main">${escapeHtml(row.name)}</span><span class="cell-sub">${escapeHtml(row.description || "")}</span></td><td class="mono">${escapeHtml(metricLabel(row.metric_type))} ${escapeHtml(row.operator)} ${row.threshold}</td><td>${row.window_minutes} min</td><td>${severityStatus(row.severity)}</td><td>${row.enabled ? status("启用") : status("停用", "off")}</td><td><div class="cell-actions"><button class="button quiet small" data-alert-action="edit" data-id="${row.id}">编辑</button><button class="button quiet small" data-alert-action="toggle" data-id="${row.id}">${row.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-alert-action="delete" data-id="${row.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
  }

  function alertEventTable(rows) {
    if (!rows.length) return emptyState("暂无事件", "规则命中后会显示在这里");
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>规则</th><th>级别</th><th>状态</th><th>指标</th><th></th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.fired_at)}</td><td><span class="cell-main">${escapeHtml(row.rule_name)}</span><span class="cell-sub">${escapeHtml(row.description)}</span></td><td>${severityStatus(row.severity)}</td><td>${row.status === "firing" ? status("告警中", "error") : status(row.status === "manual_resolved" ? "手动关闭" : "已恢复", "off")}</td><td class="mono">${Number(row.metric_value).toFixed(3)} / ${Number(row.threshold_value).toFixed(3)}</td><td>${row.status === "firing" ? `<button class="button quiet small" data-event-resolve="${row.id}">关闭</button>` : ""}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function systemLogTable(rows) {
    if (!rows.length) return emptyState("暂无日志", "管理操作和网关错误会显示在这里");
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>级别</th><th>来源</th><th>消息</th><th>请求 ID</th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td><td>${logLevelStatus(row.level)}</td><td><span class="cell-main">${escapeHtml(row.source)}</span>${row.target ? `<span class="cell-sub mono">${escapeHtml(row.target)}</span>` : ""}</td><td>${escapeHtml(row.message)}</td><td class="mono">${escapeHtml(row.request_id || "-")}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function reportTable(rows) {
    if (!rows.length) return emptyState("暂无报表", "配置收件人后可手动发送或启用定时报表");
    return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>类型</th><th>周期</th><th>状态</th><th>请求 / 错误</th><th>Token</th><th>收件人</th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td><td>${({daily: "日报", weekly: "周报", manual: "手动"})[row.report_type] || escapeHtml(row.report_type)}</td><td><span class="cell-main">${formatDate(row.period_start)}</span><span class="cell-sub">至 ${formatDate(row.period_end)}</span></td><td>${row.status === "sent" ? status("已发送") : row.status === "failed" ? status("失败", "error") : row.status === "skipped" ? status("已跳过", "off") : status("处理中", "warn")}${row.error_summary ? `<span class="cell-sub">${escapeHtml(row.error_summary)}</span>` : ""}</td><td>${formatNumber(row.metrics.request_count)} / ${formatNumber(row.metrics.error_count)}</td><td>${formatNumber(row.metrics.token_count)}</td><td>${formatNumber((row.recipients || []).length)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function openAlertRule(rule = null) {
    openModal(rule ? "编辑告警规则" : "添加告警规则", `<form id="alert-rule-form"><div class="field"><label for="alert-name">名称</label><input id="alert-name" name="name" value="${escapeHtml(rule?.name || "")}" maxlength="120" required autofocus></div><div class="field"><label for="alert-description">说明</label><input id="alert-description" name="description" value="${escapeHtml(rule?.description || "")}" maxlength="1000"></div><div class="form-grid"><div class="field"><label for="alert-metric">指标</label><select id="alert-metric" name="metric_type">${["success_rate", "error_rate", "upstream_error_rate", "request_count", "token_count", "latency_p95_ms", "active_requests", "available_accounts"].map(value => `<option value="${value}" ${rule?.metric_type === value ? "selected" : ""}>${metricLabel(value)}</option>`).join("")}</select></div><div class="field"><label for="alert-operator">比较</label><select id="alert-operator" name="operator">${[">", ">=", "<", "<=", "==", "!="].map(value => `<option value="${escapeHtml(value)}" ${rule?.operator === value ? "selected" : ""}>${escapeHtml(value)}</option>`).join("")}</select></div></div><div class="form-grid"><div class="field"><label for="alert-threshold">阈值</label><input id="alert-threshold" name="threshold" type="number" step="0.001" value="${rule?.threshold ?? 1}" required></div><div class="field"><label for="alert-window">窗口 (分钟)</label><input id="alert-window" name="window_minutes" type="number" min="1" max="1440" value="${rule?.window_minutes || 5}" required></div></div><div class="form-grid"><div class="field"><label for="alert-severity">级别</label><select id="alert-severity" name="severity"><option value="info" ${rule?.severity === "info" ? "selected" : ""}>信息</option><option value="warning" ${!rule || rule.severity === "warning" ? "selected" : ""}>警告</option><option value="critical" ${rule?.severity === "critical" ? "selected" : ""}>严重</option></select></div><div class="field"><label for="alert-cooldown">冷却 (分钟)</label><input id="alert-cooldown" name="cooldown_minutes" type="number" min="1" max="10080" value="${rule?.cooldown_minutes || 15}" required></div></div><div class="form-grid"><label class="switch-row compact"><span><strong>启用规则</strong><small>后台每分钟评估</small></span><input name="enabled" type="checkbox" ${rule?.enabled !== false ? "checked" : ""}></label><label class="switch-row compact"><span><strong>邮件通知</strong><small>使用运维设置中的收件人</small></span><input name="notify_email" type="checkbox" ${rule?.notify_email ? "checked" : ""}></label></div><p class="form-error" id="alert-rule-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-alert-rule">保存</button>`);
    modal.querySelector("#save-alert-rule").addEventListener("click", () => saveAlertRule(rule?.id));
  }

  async function saveAlertRule(id) {
    const form = modal.querySelector("#alert-rule-form");
    if (!form.reportValidity()) return;
    const values = Object.fromEntries(new FormData(form));
    values.threshold = Number(values.threshold); values.window_minutes = Number(values.window_minutes); values.cooldown_minutes = Number(values.cooldown_minutes);
    values.enabled = form.elements.enabled.checked; values.notify_email = form.elements.notify_email.checked;
    const button = modal.querySelector("#save-alert-rule"); button.disabled = true;
    try { await api(id ? `/api/admin/ops/alert-rules/${id}` : "/api/admin/ops/alert-rules", { method: id ? "PUT" : "POST", body: JSON.stringify(values) }); closeModal(); toast("告警规则已保存"); await renderRoute(); }
    catch (error) { modal.querySelector("#alert-rule-error").textContent = error.message; button.disabled = false; }
  }

  async function handleAlertAction(event) {
    const id = Number(event.currentTarget.dataset.id); const action = event.currentTarget.dataset.alertAction; const rule = rules.find(item => item.id === id); if (!rule) return;
    if (action === "edit") return openAlertRule(rule);
    if (action === "delete" && !confirm("确认删除该规则及其事件？")) return;
    try {
      if (action === "delete") await api(`/api/admin/ops/alert-rules/${id}`, { method: "DELETE" });
      else await api(`/api/admin/ops/alert-rules/${id}`, { method: "PUT", body: JSON.stringify({ ...rule, enabled: !rule.enabled }) });
      toast("告警规则已更新"); await renderRoute();
    } catch (error) { toast(error.message, true); }
  }

  async function resolveEvent(event) {
    try { await api(`/api/admin/ops/alert-events/${event.currentTarget.dataset.eventResolve}/status`, { method: "PUT", body: JSON.stringify({ status: "manual_resolved" }) }); toast("告警事件已关闭"); await renderRoute(); }
    catch (error) { toast(error.message, true); }
  }

  async function evaluateAlerts(event) {
    event.currentTarget.disabled = true;
    try { const result = await api("/api/admin/ops/evaluate", { method: "POST", body: "{}" }); toast(`评估完成：触发 ${result.data.fired}，恢复 ${result.data.resolved}`); await renderRoute(); }
    catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
  }

  function runReport() {
    openModal("发送运维报表", `<form id="ops-report-form"><div class="field"><label for="ops-report-range">统计范围</label><select id="ops-report-range" name="range"><option value="24h">近 24 小时</option><option value="7d">近 7 天</option></select></div><p class="field-hint">报表将发送至运维设置中的报表收件人。</p><p class="form-error" id="ops-report-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="send-ops-report">发送</button>`);
    modal.querySelector("#send-ops-report").addEventListener("click", sendReport);
  }

  async function sendReport(event) {
    event.currentTarget.disabled = true;
    const range = modal.querySelector("#ops-report-range").value;
    try {
      await api("/api/admin/ops/reports/run", { method: "POST", body: JSON.stringify({ range }) });
      closeModal(); toast("运维报表已发送"); await renderRoute();
    } catch (error) { modal.querySelector("#ops-report-error").textContent = error.message; event.currentTarget.disabled = false; }
  }

  function openSettings() {
    openModal("运维设置", `<form id="ops-settings-form">
      <div class="form-grid"><div class="field"><label for="ops-refresh-seconds">自动刷新 (秒)</label><input id="ops-refresh-seconds" name="auto_refresh_seconds" type="number" min="5" max="300" value="${settings.auto_refresh_seconds}" required></div><div class="field"><label for="ops-retention">请求保留 (天)</label><input id="ops-retention" name="request_retention_days" type="number" min="1" max="3650" value="${settings.request_retention_days}" required></div></div>
      <div class="form-grid"><div class="field"><label for="ops-recipients">告警收件人</label><textarea id="ops-recipients" name="alert_recipients" class="compact-textarea" placeholder="每行一个邮箱">${escapeHtml((settings.alert_recipients || []).join("\n"))}</textarea></div><div class="field"><label for="ops-report-recipients">报表收件人</label><textarea id="ops-report-recipients" name="report_recipients" class="compact-textarea" placeholder="每行一个邮箱">${escapeHtml((settings.report_recipients || []).join("\n"))}</textarea></div></div>
      <span class="field-hint">${settings.mail_configured ? "邮件投递已配置" : "当前未配置邮件投递"}</span>
      <div class="form-grid"><label class="switch-row compact"><span><strong>启用告警邮件</strong><small>仅启用邮件通知的规则发送</small></span><input name="email_enabled" type="checkbox" ${settings.email_enabled ? "checked" : ""}></label><label class="switch-row compact"><span><strong>启用日报</strong><small>按 UTC cron 发送</small></span><input name="daily_report_enabled" type="checkbox" ${settings.daily_report_enabled ? "checked" : ""}></label></div>
      <div class="form-grid"><div class="field"><label for="ops-daily-cron">日报 cron</label><input id="ops-daily-cron" name="daily_report_cron" class="mono" value="${escapeHtml(settings.daily_report_cron)}" required></div><div class="field"><label for="ops-weekly-cron">周报 cron</label><input id="ops-weekly-cron" name="weekly_report_cron" class="mono" value="${escapeHtml(settings.weekly_report_cron)}" required></div></div>
      <label class="switch-row compact"><span><strong>启用周报</strong><small>按 UTC cron 发送</small></span><input name="weekly_report_enabled" type="checkbox" ${settings.weekly_report_enabled ? "checked" : ""}></label>
      <div class="form-grid"><div class="field"><label for="ops-log-level">运行日志级别</label><select id="ops-log-level" name="runtime_log_level">${["error", "warn", "info", "debug", "trace"].map(level => `<option value="${level}" ${settings.runtime_log.level === level ? "selected" : ""}>${level.toUpperCase()}</option>`).join("")}</select></div><label class="switch-row compact"><span><strong>运行日志写入 SQLite</strong><small>关闭后仍输出容器标准日志</small></span><input name="runtime_log_db_enabled" type="checkbox" ${settings.runtime_log.db_enabled ? "checked" : ""}></label></div>
      <p class="form-error" id="ops-settings-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-ops-settings">保存</button>`);
    modal.querySelector("#save-ops-settings").addEventListener("click", saveSettings);
  }

  async function saveSettings(event) {
    const form = modal.querySelector("#ops-settings-form"); if (!form.reportValidity()) return; event.currentTarget.disabled = true;
    const values = Object.fromEntries(new FormData(form));
    const opsValues = {
      auto_refresh_seconds: Number(values.auto_refresh_seconds), request_retention_days: Number(values.request_retention_days),
      alert_recipients: parseModelList(values.alert_recipients), email_enabled: form.elements.email_enabled.checked,
      report_recipients: parseModelList(values.report_recipients), daily_report_enabled: form.elements.daily_report_enabled.checked,
      daily_report_cron: values.daily_report_cron.trim(), weekly_report_enabled: form.elements.weekly_report_enabled.checked,
      weekly_report_cron: values.weekly_report_cron.trim(),
    };
    try {
      await api("/api/admin/ops/settings", { method: "PUT", body: JSON.stringify(opsValues) });
      await api("/api/admin/ops/runtime-log-config", { method: "PUT", body: JSON.stringify({ level: values.runtime_log_level, db_enabled: form.elements.runtime_log_db_enabled.checked }) });
      closeModal(); toast("运维设置已保存"); await renderRoute();
    }
    catch (error) { modal.querySelector("#ops-settings-error").textContent = error.message; event.currentTarget.disabled = false; }
  }

  function startLiveMetrics() {
    if (!window.EventSource) return;
    liveSource = new EventSource("/api/admin/ops/live");
    liveSource.addEventListener("metrics", event => {
      const data = JSON.parse(event.data).data; if (!data) return;
      setLiveValue("ops-live-qps", formatRate(data.qps)); setLiveValue("ops-live-tps", formatRate(data.tps));
      setLiveValue("ops-live-active", formatNumber(data.active_gateway_requests)); setLiveValue("ops-live-ttft", `${formatNumber(data.average_ttft_ms)} ms`);
      setLiveValue("ops-live-switches", formatNumber(data.account_switches)); setLiveValue("ops-live-attempts", formatNumber(data.upstream_attempts));
    });
  }

  function setLiveValue(id, value) { const element = document.getElementById(id); if (element) element.textContent = value; }
  function stopLiveMetrics() { if (liveSource) liveSource.close(); liveSource = null; }

  function scheduleRefresh() {
    stopRefresh();
    const seconds = Math.max(5, Number(settings?.auto_refresh_seconds || 10));
    refreshTimer = setTimeout(() => { if (currentRouteName() === "opsAdmin") renderRoute(); }, seconds * 1000);
  }
  function stopRefresh() { if (refreshTimer) clearTimeout(refreshTimer); refreshTimer = null; }

  function metricLabel(value) { return ({ success_rate: "成功率 (%)", error_rate: "错误率 (%)", upstream_error_rate: "上游错误率 (%)", request_count: "请求数", token_count: "Token 数", latency_p95_ms: "P95 延迟 (ms)", active_requests: "活跃请求", available_accounts: "可用账号数" })[value] || value; }
  function severityStatus(value) { return value === "critical" ? status("严重", "error") : value === "warning" ? status("警告", "warn") : status("信息"); }
  function logLevelStatus(value) { return value === "error" ? status("ERROR", "error") : value === "warn" ? status("WARN", "warn") : value === "debug" || value === "trace" ? status(value.toUpperCase(), "off") : status("INFO"); }
  function rangeLabel(value) { return ({ "5m": "近 5 分钟", "30m": "近 30 分钟", "1h": "近 1 小时", "6h": "近 6 小时", "24h": "近 24 小时", "7d": "近 7 天" })[value] || value; }
  function formatRate(value) { return Number(value || 0).toLocaleString("zh-CN", { maximumFractionDigits: 3 }); }
  function formatBytes(value) { if (value == null) return "-"; const units = ["B", "KB", "MB", "GB"]; let size = Number(value); let unit = 0; while (size >= 1024 && unit < units.length - 1) { size /= 1024; unit++; } return `${size.toFixed(unit > 1 ? 2 : 0)} ${units[unit]}`; }
  function formatDuration(seconds) { const value = Number(seconds || 0); if (value < 60) return `${value}s`; if (value < 3600) return `${Math.floor(value / 60)}m`; if (value < 86400) return `${Math.floor(value / 3600)}h ${Math.floor(value % 3600 / 60)}m`; return `${Math.floor(value / 86400)}d ${Math.floor(value % 86400 / 3600)}h`; }

  window.addEventListener("hashchange", () => { if (currentRouteName() !== "opsAdmin") { stopRefresh(); stopLiveMetrics(); } });
  window.Sub2MiniOps = { render };
})();
