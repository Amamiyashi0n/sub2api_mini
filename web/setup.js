"use strict";

window.Sub2MiniSetup = (() => {
  const labels = {
    configuration_loaded: "环境配置",
    master_key_loaded: "凭证主密钥",
    database_connected: "SQLite 连接",
    data_directory_ready: "持久化目录",
    sqlite_wal: "WAL 日志模式",
    foreign_keys: "外键约束",
    admin_configured: "管理员账户",
    single_process_runtime: "单进程运行",
    redis_required: "Redis 依赖",
  };

  async function render() {
    const result = await api("/setup/status");
    const data = result.data;
    const checks = Object.entries(data.checks);
    app.innerHTML = `<main class="setup-screen"><header class="setup-header"><img src="/logo.svg" alt=""><div><span>SUB2API MINI</span><h1>部署检查</h1><p>当前实例由环境文件初始化，SQLite 与运行组件已内置。</p></div></header>
      <section class="setup-summary"><div><span>状态</span><strong>${data.needs_setup ? "需要初始化" : "已就绪"}</strong></div><div><span>版本</span><strong>${escapeHtml(data.version)}</strong></div><div><span>数据库迁移</span><strong>v${data.database.migration_version}</strong></div><div><span>连接池</span><strong>${data.database.max_connections}</strong></div></section>
      <section class="setup-checks"><div class="section-title"><h2>运行前置条件</h2><button class="button secondary" id="refresh-setup">重新检查</button></div><div class="setup-check-grid">${checks.map(([key, ok]) => `<article class="${ok || key === "redis_required" ? "ready" : "failed"}"><span>${escapeHtml(labels[key] || key)}</span><strong>${key === "redis_required" ? (ok ? "需要" : "无需") : (ok ? "通过" : "异常")}</strong></article>`).join("")}</div></section>
      <section class="setup-runtime"><div><h2>SQLite 运行参数</h2><dl class="detail-list"><div><dt>数据库引擎</dt><dd>${escapeHtml(data.database.engine)}</dd></div><div><dt>日志模式</dt><dd>${escapeHtml(data.database.journal_mode.toUpperCase())}</dd></div><div><dt>初始化方式</dt><dd>权限受控的环境文件</dd></div></dl></div><div><h2>监听端点</h2><dl class="detail-list"><div><dt>主服务</dt><dd class="mono">${escapeHtml(data.listeners.main)}</dd></div><div><dt>OAuth 回调</dt><dd class="mono">${escapeHtml(data.listeners.oauth_callback)}</dd></div><div><dt>运行模型</dt><dd>Axum + SQLite · 单进程</dd></div></dl></div></section>
      <footer class="setup-actions"><a class="button" href="#/overview">${state.user ? "返回控制台" : "进入登录"}</a><a class="button secondary" href="#/status">服务状态</a></footer></main>`;
    document.querySelector("#refresh-setup").addEventListener("click", render);
  }

  return { render };
})();
