"use strict";

window.Sub2MiniUsage = (() => {
  const errorLabels = {
    success: "成功",
    rate_limited: "限流 (429)",
    client_error: "客户端错误 (4xx)",
    upstream_error: "上游错误 (5xx)",
    transport_error: "传输错误",
  };

  function params(includePage = false) {
    const result = new URLSearchParams();
    if (includePage) {
      result.set("page", String(usagePage));
      result.set("page_size", "25");
    }
    Object.entries(usageFilters).forEach(([key, value]) => {
      if (value !== "" && value != null) result.set(key, String(value));
    });
    return result;
  }

  function option(value, label, current) {
    return `<option value="${escapeHtml(value)}" ${String(current || "") === value ? "selected" : ""}>${escapeHtml(label)}</option>`;
  }

  function bars(rows, valueKey, labelKey = "name") {
    if (!rows.length) return emptyState("暂无分布数据", "所选范围内没有匹配请求");
    const max = Math.max(...rows.map(row => Number(row[valueKey]) || 0), 1);
    return `<div class="usage-bars">${rows.map(row => `<div><span title="${escapeHtml(row[labelKey])}">${escapeHtml(row[labelKey])}</span><div><i style="width:${Math.max(2, (Number(row[valueKey]) || 0) / max * 100)}%"></i></div><strong>${formatNumber(row[valueKey])}</strong></div>`).join("")}</div>`;
  }

  async function render(page) {
    const query = params(true);
    const statsQuery = params(false);
    const [result, keys, analytics] = await Promise.all([
      api(`${roleApiBase()}/usage?${query}`),
      api(`${roleApiBase()}/keys`),
      api(`${roleApiBase()}/usage/stats?${statsQuery}`),
    ]);
    const pages = Math.max(1, Math.ceil(result.meta.total / result.meta.page_size));
    const summary = analytics.data.summary;
    const actions = `<button class="button secondary" id="export-usage">导出 CSV</button>${state.role === "admin" ? '<button class="button danger secondary" id="cleanup-usage">清理</button>' : ""}<button class="button secondary" id="refresh-usage">刷新</button>`;
    page.innerHTML = `${pageHeader("使用日志", `${result.meta.total} 条记录`, actions)}
      <section class="metric-grid usage-metrics">
        ${metric("请求", formatNumber(summary.requests), "good")}
        ${metric("失败", formatNumber(summary.failed_requests), summary.failed_requests ? "warn" : "good")}
        ${metric("总 Token", formatNumber(summary.total_tokens))}
        ${metric("缓存 Token", formatNumber(summary.cached_input_tokens))}
        ${metric("缓存写入", formatNumber(summary.cache_write_tokens))}
        ${metric("图片 Token", `${formatNumber(summary.image_input_tokens)} / ${formatNumber(summary.image_output_tokens)}`)}
        ${metric("推理 Token", formatNumber(summary.reasoning_tokens))}
        ${metric("成本", formatUsdMicros(summary.cost_microusd))}
        ${metric("平均耗时", `${formatNumber(summary.average_duration_ms)} ms`)}
        ${metric("最长耗时", `${formatNumber(summary.maximum_duration_ms)} ms`, summary.maximum_duration_ms > 30000 ? "warn" : "")}
      </section>
      <form id="usage-filter-form" class="filter-bar usage-filter-bar">
        ${state.role === "admin" ? `<div class="field"><label for="usage-filter-user">用户 ID</label><input id="usage-filter-user" name="user_id" type="number" min="1" value="${escapeHtml(usageFilters.user_id || "")}" placeholder="全部"></div>` : ""}
        <div class="field"><label for="usage-filter-key">API Key</label><select id="usage-filter-key" name="api_key_id"><option value="">全部</option>${keys.data.map(key => option(String(key.id), key.name, usageFilters.api_key_id)).join("")}</select></div>
        <div class="field"><label for="usage-filter-model">模型</label><input id="usage-filter-model" name="model" value="${escapeHtml(usageFilters.model || "")}" placeholder="包含文本"></div>
        <div class="field"><label for="usage-filter-endpoint">端点</label><select id="usage-filter-endpoint" name="endpoint"><option value="">全部</option>${["/v1/responses", "/v1/chat/completions", "/v1/models"].map(value => option(value, value, usageFilters.endpoint)).join("")}</select></div>
        <div class="field"><label for="usage-filter-status-class">状态分类</label><select id="usage-filter-status-class" name="status_class"><option value="">全部</option>${[["success", "成功"], ["error", "全部错误"], ["4xx", "4xx"], ["429", "429"], ["5xx", "5xx"]].map(([value, label]) => option(value, label, usageFilters.status_class)).join("")}</select></div>
        <div class="field"><label for="usage-filter-status">状态码</label><input id="usage-filter-status" name="status_code" type="number" min="100" max="599" value="${escapeHtml(usageFilters.status_code || "")}" placeholder="例如 200"></div>
        <div class="field"><label for="usage-filter-type">请求类型</label><select id="usage-filter-type" name="request_type"><option value="">全部</option>${option("sync", "同步", usageFilters.request_type)}${option("stream", "流式", usageFilters.request_type)}</select></div>
        <div class="field"><label for="usage-filter-tier">服务层级</label><input id="usage-filter-tier" name="service_tier" value="${escapeHtml(usageFilters.service_tier || "")}" placeholder="default / priority"></div>
        <div class="field"><label for="usage-filter-start">开始日期</label><input id="usage-filter-start" name="start_date" type="date" value="${escapeHtml(usageFilters.start_date || "")}"></div>
        <div class="field"><label for="usage-filter-end">结束日期</label><input id="usage-filter-end" name="end_date" type="date" value="${escapeHtml(usageFilters.end_date || "")}"></div>
        <div class="filter-actions"><button class="button" type="submit">筛选</button><button class="button secondary" type="button" id="clear-usage-filter">清除</button></div>
      </form>
      <div class="usage-analytics-grid">
        <section><div class="section-title"><h2>模型请求</h2></div>${bars(analytics.data.models, "requests", "model")}</section>
        <section><div class="section-title"><h2>错误分类</h2></div>${bars(analytics.data.errors.map(row => ({ ...row, name: errorLabels[row.name] || row.name })), "requests")}</section>
        <section><div class="section-title"><h2>请求类型</h2></div>${bars(analytics.data.request_types, "requests")}</section>
        <section><div class="section-title"><h2>服务层级</h2></div>${bars(analytics.data.service_tiers, "requests")}</section>
      </div>
      ${usageTable(result.data, true)}
      <nav class="pagination" aria-label="使用日志分页"><button class="button secondary" id="usage-prev" ${usagePage <= 1 ? "disabled" : ""}>上一页</button><span>第 ${usagePage} / ${pages} 页</span><button class="button secondary" id="usage-next" ${usagePage >= pages ? "disabled" : ""}>下一页</button></nav>`;
    page.querySelector("#refresh-usage").addEventListener("click", renderRoute);
    page.querySelector("#export-usage").addEventListener("click", exportCsv);
    page.querySelector("#cleanup-usage")?.addEventListener("click", previewCleanup);
    page.querySelector("#usage-filter-form").addEventListener("submit", event => {
      event.preventDefault();
      usageFilters = Object.fromEntries([...new FormData(event.currentTarget)].filter(([, value]) => value !== ""));
      usagePage = 1;
      renderRoute();
    });
    page.querySelector("#clear-usage-filter").addEventListener("click", () => { usageFilters = {}; usagePage = 1; renderRoute(); });
    page.querySelector("#usage-prev").addEventListener("click", () => { usagePage -= 1; renderRoute(); });
    page.querySelector("#usage-next").addEventListener("click", () => { usagePage += 1; renderRoute(); });
    page.querySelectorAll("[data-usage-detail]").forEach(button => button.addEventListener("click", openUsageDetail));
  }

  async function exportCsv(event) {
    event.currentTarget.disabled = true;
    try {
      const response = await fetch(`${API}${roleApiBase()}/usage/export?${params(false)}`, { credentials: "include" });
      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body.error?.message || `导出失败 (${response.status})`);
      }
      const blob = await response.blob();
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `sub2api-mini-usage-${new Date().toISOString().slice(0, 10)}.csv`;
      link.click();
      URL.revokeObjectURL(url);
      toast("用量 CSV 已导出");
    } catch (error) { toast(error.message, true); }
    finally { event.currentTarget.disabled = false; }
  }

  function cleanupFilter() {
    const value = { ...usageFilters };
    for (const key of ["user_id", "api_key_id", "status_code"]) {
      if (value[key]) value[key] = Number(value[key]);
    }
    if (value.stream === "true") value.stream = true;
    if (value.stream === "false") value.stream = false;
    return value;
  }

  async function previewCleanup() {
    if (!usageFilters.end_date) {
      toast("清理前请先在筛选中设置结束日期", true);
      return;
    }
    try {
      const preview = await api("/api/admin/usage/cleanup/preview", { method: "POST", body: JSON.stringify(cleanupFilter()) });
      const data = preview.data;
      openModal("确认清理使用日志", `<div class="cleanup-preview"><p>快照中匹配 <strong>${formatNumber(data.matched_count)}</strong> 条日志。</p><dl class="detail-list"><div><dt>结束日期</dt><dd>${escapeHtml(data.filter.end_date)}</dd></div><div><dt>快照最大 ID</dt><dd class="mono">${data.snapshot_max_id}</dd></div><div><dt>确认有效期</dt><dd>${formatDate(data.expires_at)}</dd></div></dl><p class="form-error" id="usage-cleanup-error"></p></div>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-usage-cleanup" ${data.matched_count ? "" : "disabled"}>删除 ${formatNumber(data.matched_count)} 条</button>`);
      modal.querySelector("#confirm-usage-cleanup")?.addEventListener("click", async event => {
        event.currentTarget.disabled = true;
        try {
          const result = await api("/api/admin/usage/cleanup/confirm", { method: "POST", body: JSON.stringify({ filter: data.filter, snapshot_max_id: data.snapshot_max_id, filter_hash: data.filter_hash, confirmation_token: data.confirmation_token, confirm: true }) });
          closeModal();
          toast(`已删除 ${formatNumber(result.data.deleted_rows)} 条使用日志`);
          usagePage = 1;
          await renderRoute();
        } catch (error) {
          modal.querySelector("#usage-cleanup-error").textContent = error.message;
          event.currentTarget.disabled = false;
        }
      });
    } catch (error) { toast(error.message, true); }
  }

  return { render };
})();
