"use strict";

window.Sub2MiniContent = (() => {
  const monitorIntervals = new Set([30, 60, 120]);
  let monitorTimer = null;
  let countdownTimer = null;

  function stopMonitorTimers() {
    clearTimeout(monitorTimer);
    clearInterval(countdownTimer);
    monitorTimer = null;
    countdownTimer = null;
  }

  function pageLinks(items) {
    return `<div class="public-page-links">${items.map(item => `<a href="#/page/${encodeURIComponent(item.slug)}"><span>${item.kind === "legal" ? "法律文档" : item.render_mode === "iframe" ? "嵌入页面" : "页面"}</span><strong>${escapeHtml(item.title)}</strong></a>`).join("")}</div>`;
  }

  async function renderPage(page, slug) {
    stopMonitorTimers();
    page.innerHTML = `<div class="boot-screen compact"><p>正在载入</p></div>`;
    try {
      const path = encodeURIComponent(decodeURIComponent(slug));
      const result = await api(`/api/user/pages/${path}`);
      const data = result.data;
      if (data.render_mode === "iframe") {
        page.innerHTML = `<section class="embedded-page"><header><div><span>嵌入页面</span><h1>${escapeHtml(data.title)}</h1><p>更新于 ${formatDate(data.updated_at)}</p></div><a class="button secondary" href="${escapeHtml(data.iframe_url)}" target="_blank" rel="noopener noreferrer">新窗口打开</a></header><iframe src="${escapeHtml(data.iframe_url)}" title="${escapeHtml(data.title)}" allowfullscreen></iframe></section>`;
        return;
      }
      page.innerHTML = `<article class="public-document"><header><span>${data.kind === "legal" ? "法律文档" : "内容页"}</span><h1>${escapeHtml(data.title)}</h1><p>更新于 ${formatDate(data.updated_at)}</p></header><div class="document-layout"><aside class="document-toc" hidden><strong>目录</strong><nav></nav></aside><div class="markdown-body document-body">${data.rendered_html || ""}</div></div></article>`;
      buildToc(page.querySelector(".public-document"));
      enhanceMarkdown(page);
    } catch (error) {
      page.innerHTML = emptyState("页面不存在", error.message, "返回内容页", "back-content-pages");
      page.querySelector("#back-content-pages")?.addEventListener("click", () => { location.hash = "#/pages"; });
    }
  }

  function buildToc(root) {
    const body = root.querySelector(".document-body");
    const aside = root.querySelector(".document-toc");
    const nav = aside?.querySelector("nav");
    if (!body || !aside || !nav) return;
    const headings = [...body.querySelectorAll("h1, h2, h3, h4")];
    if (!headings.length) return;
    const used = new Set();
    headings.forEach((heading, index) => {
      const base = heading.textContent.trim().toLowerCase().replace(/[^a-z0-9\u4e00-\u9fff]+/g, "-").replace(/^-|-$/g, "") || `section-${index + 1}`;
      let id = base;
      let suffix = 2;
      while (used.has(id)) id = `${base}-${suffix++}`;
      used.add(id);
      heading.id = id;
      const button = document.createElement("button");
      button.type = "button";
      button.className = `toc-link level-${heading.tagName.slice(1)}`;
      button.textContent = heading.textContent;
      button.addEventListener("click", () => heading.scrollIntoView({ behavior: "smooth", block: "start" }));
      nav.append(button);
    });
    aside.hidden = false;
  }

  function enhanceMarkdown(root) {
    root.querySelectorAll(".markdown-body a").forEach(link => {
      try {
        const target = new URL(link.href, location.href);
        if (target.origin !== location.origin) {
          link.target = "_blank";
          link.rel = "noopener noreferrer";
        }
      } catch (_) {}
    });
    root.querySelectorAll(".markdown-body pre").forEach(pre => {
      if (pre.querySelector(".copy-code")) return;
      const button = document.createElement("button");
      button.type = "button";
      button.className = "copy-code";
      button.textContent = "复制";
      button.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(pre.querySelector("code")?.textContent || pre.textContent || "");
          button.textContent = "已复制";
        } catch (_) {
          button.textContent = "复制失败";
        }
        setTimeout(() => { if (button.isConnected) button.textContent = "复制"; }, 1600);
      });
      pre.append(button);
    });
  }

  async function renderPages(page) {
    stopMonitorTimers();
    const result = await api("/api/user/pages");
    page.innerHTML = `${pageHeader("内容页", `${result.data.length} 个可用页面`)}${result.data.length ? pageLinks(result.data) : emptyState("暂无内容页", "管理员发布的内容会显示在这里")}`;
  }

  function timeline(item) {
    const points = Array.isArray(item.timeline) ? item.timeline : [];
    if (!points.length) return '<div class="monitor-timeline empty"><span>暂无探测历史</span></div>';
    return `<div class="monitor-timeline" role="img" aria-label="最近 ${points.length} 次探测">${points.map(point => `<span class="${escapeHtml(point.status || "error")}" title="${escapeHtml(`${formatDate(point.checked_at)} · ${monitorStatusText(point.status)} · 业务 ${point.latency_ms == null ? "-" : `${point.latency_ms} ms`} · Ping ${point.ping_latency_ms == null ? "-" : `${point.ping_latency_ms} ms`}`)}"></span>`).join("")}</div>`;
  }

  function readMonitorInterval() {
    const value = Number(localStorage.getItem("mini_monitor_interval") || 60);
    return monitorIntervals.has(value) ? value : 60;
  }

  function scheduleMonitor(page, seconds) {
    stopMonitorTimers();
    let remaining = seconds;
    const label = page.querySelector("#monitor-countdown");
    const update = () => { if (label) label.textContent = `${remaining} 秒后刷新`; };
    update();
    countdownTimer = setInterval(() => {
      remaining = Math.max(0, remaining - 1);
      update();
    }, 1000);
    monitorTimer = setTimeout(() => {
      clearInterval(countdownTimer);
      countdownTimer = null;
      monitorTimer = null;
      if (page.isConnected && currentRouteName() === "monitor") renderMonitor(page).catch(error => toast(error.message, true));
    }, seconds * 1000);
  }

  async function renderMonitor(page) {
    stopMonitorTimers();
    const result = await api("/api/user/channel-monitors");
    const items = result.data;
    const healthy = items.filter(item => item.primary_status === "operational").length;
    const degraded = items.filter(item => item.primary_status && item.primary_status !== "operational").length;
    const interval = readMonitorInterval();
    page.innerHTML = `${pageHeader("频道状态", items.length ? `${healthy}/${items.length} 个频道正常` : "尚未配置监控", `<div class="monitor-refresh-controls"><span id="monitor-countdown"></span><select id="monitor-refresh-interval" aria-label="自动刷新周期"><option value="30" ${interval === 30 ? "selected" : ""}>30 秒</option><option value="60" ${interval === 60 ? "selected" : ""}>60 秒</option><option value="120" ${interval === 120 ? "selected" : ""}>120 秒</option></select><button class="button secondary" id="refresh-channel-monitor">刷新</button></div>`)}
      <section class="metric-grid">${metric("监控频道", items.length)}${metric("正常", healthy, "good")}${metric("异常", degraded, degraded ? "warn" : "good")}</section>
      ${items.length ? `<div class="monitor-grid">${items.map(item => `<article class="monitor-card"><header><div><h2>${escapeHtml(item.name)}</h2><p>${escapeHtml(item.group_name || item.provider.toUpperCase())}</p></div>${monitorStatus(item.primary_status)}</header><div class="monitor-primary"><strong>${escapeHtml(item.primary_model)}</strong><span>业务 ${item.primary_latency_ms == null ? "-" : `${item.primary_latency_ms} ms`} · Ping ${item.primary_ping_latency_ms == null ? "-" : `${item.primary_ping_latency_ms} ms`}</span></div>${timeline(item)}<div class="availability-row"><span>近 7 天可用率</span><strong>${Number(item.availability_7d).toFixed(2)}%</strong></div>${item.extra_models.length ? `<div class="model-cloud compact">${item.extra_models.map(model => `<code>${escapeHtml(model.model)} · ${escapeHtml(monitorStatusText(model.status))}</code>`).join("")}</div>` : ""}<button class="button quiet small" data-monitor-detail="${item.id}">查看详情</button></article>`).join("")}</div>` : emptyState("暂无频道监控", "管理员创建监控后会显示可用率与延迟")}`;
    page.querySelector("#refresh-channel-monitor")?.addEventListener("click", () => renderMonitor(page).catch(error => toast(error.message, true)));
    page.querySelector("#monitor-refresh-interval")?.addEventListener("change", event => {
      const seconds = Number(event.currentTarget.value);
      if (!monitorIntervals.has(seconds)) return;
      localStorage.setItem("mini_monitor_interval", String(seconds));
      scheduleMonitor(page, seconds);
    });
    page.querySelectorAll("[data-monitor-detail]").forEach(button => button.addEventListener("click", openMonitorDetail));
    scheduleMonitor(page, interval);
  }

  async function openMonitorDetail(event) {
    try {
      const result = await api(`/api/user/channel-monitors/${event.currentTarget.dataset.monitorDetail}/status`);
      const data = result.data;
      openModal(`${data.name} · 可用率`, `<div class="table-wrap"><table><thead><tr><th>模型</th><th>当前</th><th>最新业务 / Ping</th><th>7 天</th><th>15 天</th><th>30 天</th><th>7 天平均业务 / Ping</th></tr></thead><tbody>${data.models.map(model => `<tr><td class="mono">${escapeHtml(model.model)}</td><td>${monitorStatus(model.latest_status)}</td><td>${model.latest_latency_ms == null ? "-" : `${model.latest_latency_ms} ms`} / ${model.latest_ping_latency_ms == null ? "-" : `${model.latest_ping_latency_ms} ms`}</td><td>${Number(model.availability_7d).toFixed(2)}%</td><td>${Number(model.availability_15d).toFixed(2)}%</td><td>${Number(model.availability_30d).toFixed(2)}%</td><td>${model.avg_latency_7d_ms == null ? "-" : `${model.avg_latency_7d_ms} ms`} / ${model.avg_ping_latency_7d_ms == null ? "-" : `${model.avg_ping_latency_7d_ms} ms`}</td></tr>`).join("")}</tbody></table></div>`, `<button class="button" data-close-modal>关闭</button>`);
    } catch (error) {
      toast(error.message, true);
    }
  }

  window.addEventListener("hashchange", () => {
    if (currentRouteName() !== "monitor") stopMonitorTimers();
  });

  return { renderPage, renderPages, renderMonitor };
})();
