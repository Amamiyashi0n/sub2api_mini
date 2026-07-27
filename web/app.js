"use strict";
const API = location.origin;
const THEME_STORAGE_KEY = "mini_theme";
const initialTheme = normalizeTheme(localStorage.getItem(THEME_STORAGE_KEY));
document.documentElement.dataset.theme = initialTheme;
const state = {
csrf: sessionStorage.getItem("mini_csrf") || "",
user: null,
displayName: null,
role: null,
siteName: "Sub2API Mini",
siteSubtitle: "个人 AI API 网关",
siteLogo: "/logo.svg",
version: "",
defaultTheme: "light",
theme: initialTheme,
contactInfo: "",
docUrl: "",
homeContent: "",
homeContentHtml: "",
homeContentUrl: null,
registrationEnabled: false,
emailVerificationEnabled: false,
passwordResetEnabled: true,
mailConfigured: false,
turnstileEnabled: false,
turnstileSiteKey: "",
};
const app = document.querySelector("#app");
const modal = document.querySelector("#modal");
const toastRegion = document.querySelector("#toast-region");
const routes = {
overview: { label: "概览", render: renderOverview },
opsAdmin: { label: "运行运维", render: renderOpsAdmin, adminOnly: true },
users: { label: "用户管理", render: renderUsers, adminOnly: true },
accounts: { label: "账号管理", render: renderAccounts, adminOnly: true },
proxies: { label: "网络代理", render: renderProxies, adminOnly: true },
keys: { label: "API Key", render: renderKeys },
batchImages: { label: "批量生图", render: renderBatchImages },
models: { label: "可用模型", render: renderModels },
channels: { label: "可用频道", render: renderAvailableChannels },
monitor: { label: "频道状态", render: renderChannelMonitor },
usage: { label: "使用日志", render: renderUsage },
announcements: { label: "公告", render: renderAnnouncements },
pages: { label: "内容页", render: renderPages },
subscriptions: { label: "我的订阅", render: renderSubscriptionsFeature },
redeem: { label: "兑换码", render: renderRedeem },
content: { label: "内容管理", render: renderContentAdmin, adminOnly: true },
settings: { label: "运行设置", render: renderSettings, adminOnly: true },
audit: { label: "审计日志", render: renderAudit, adminOnly: true },
groups: { label: "路由分组", render: renderGroupAdmin, adminOnly: true },
channelsAdmin: { label: "频道定价", render: renderChannelAdminFeature, adminOnly: true },
plans: { label: "套餐管理", render: renderPlanAdminFeature, adminOnly: true },
redeemAdmin: { label: "兑换码管理", render: renderRedeemAdmin, adminOnly: true },
ordersAdmin: { label: "订单管理", render: renderOrderAdmin, adminOnly: true },
monitorAdmin: { label: "频道监控", render: renderChannelMonitorAdmin, adminOnly: true },
riskAdmin: { label: "风险控制", render: renderRiskControlAdmin, adminOnly: true },
promptAuditAdmin: { label: "Prompt 审计", render: renderPromptAuditAdmin, adminOnly: true },
profile: { label: "个人资料", render: renderProfile },
"email-verify": { label: "验证邮箱", render: renderEmailVerification, hidden: true },
status: { label: "服务状态", render: renderStatus },
};
const routeIcons = {
overview: "grid", opsAdmin: "activity", users: "users", accounts: "globe",
proxies: "server", keys: "key", batchImages: "image", models: "box",
channels: "radio", monitor: "activity", usage: "chart", announcements: "bell",
pages: "file", subscriptions: "badge", redeem: "ticket", content: "bell",
settings: "settings", audit: "shield", groups: "folder", channelsAdmin: "tag",
plans: "badge", redeemAdmin: "ticket", ordersAdmin: "receipt",
monitorAdmin: "activity", riskAdmin: "shield", promptAuditAdmin: "fileSearch",
profile: "user", status: "activity",
};
const iconShapes = {
grid: '<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>',
activity: '<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>',
users: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/>',
globe: '<circle cx="12" cy="12" r="10"/><path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10Z"/>',
server: '<rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/>',
key: '<circle cx="7.5" cy="15.5" r="5.5"/><path d="m21 2-9.6 9.6M15 5l4 4"/>',
image: '<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21"/>',
box: '<path d="m21 16-4 4-4-4M17 20V4M3 8l4-4 4 4M7 4v16"/>',
radio: '<path d="M4.9 19.1C1 15.2 1 8.8 4.9 4.9M7.8 16.2a6 6 0 0 1 0-8.5M19.1 4.9c3.9 3.9 3.9 10.3 0 14.2M16.2 7.8a6 6 0 0 1 0 8.5"/><circle cx="12" cy="12" r="2"/>',
chart: '<path d="M3 3v18h18M7 16l4-4 4 3 5-7"/>',
bell: '<path d="M10.3 21a2 2 0 0 0 3.4 0M18 8A6 6 0 0 0 6 8c0 7-3 7-3 9h18c0-2-3-2-3-9"/>',
file: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/><polyline points="14 2 14 8 20 8"/>',
badge: '<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a2 2 0 0 0-1.17 1.51L.5 16.9a2 2 0 0 0 .83 1.83l7.64 5.08a2 2 0 0 0 2.06 0l7.64-5.08a2 2 0 0 0 .83-1.83l-.93-9.31a2 2 0 0 0-1.17-1.51l-8.57-3.9Z" transform="scale(.9) translate(1.3 0)"/><path d="m9 12 2 2 4-4"/>',
ticket: '<path d="M2 9a3 3 0 0 0 0 6v2a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-2a3 3 0 0 0 0-6V7a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2Z"/><path d="M13 5v2M13 17v2M13 11v2"/>',
settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.09a2 2 0 0 1-1-1.74v-.51a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z"/><circle cx="12" cy="12" r="3"/>',
shield: '<path d="M20 13c0 5-3.5 7.5-8 9-4.5-1.5-8-4-8-9V5l8-3 8 3Z"/><path d="m9 12 2 2 4-4"/>',
folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9l-.8-1.2A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
tag: '<path d="M12.6 2.4a2 2 0 0 0-1.4-.6H4a2 2 0 0 0-2 2V11a2 2 0 0 0 .6 1.4l8 8a2 2 0 0 0 2.8 0l7-7a2 2 0 0 0 0-2.8Z"/><circle cx="7" cy="7" r="1"/>',
receipt: '<path d="M4 2v20l2-1 2 1 2-1 2 1 2-1 2 1 2-1 2 1V2l-2 1-2-1-2 1-2-1-2 1-2-1-2 1Z"/><path d="M16 8h-6M16 12h-6M13 16h-3"/>',
fileSearch: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7"/><polyline points="14 2 14 8 20 8"/><circle cx="18" cy="18" r="3"/><path d="m20.2 20.2 1.8 1.8"/>',
user: '<circle cx="12" cy="8" r="4"/><path d="M4 22a8 8 0 0 1 16 0"/>',
menu: '<line x1="4" x2="20" y1="6" y2="6"/><line x1="4" x2="20" y1="12" y2="12"/><line x1="4" x2="20" y1="18" y2="18"/>',
chevron: '<path d="m9 18 6-6-6-6"/>',
chevronDown: '<path d="m6 9 6 6 6-6"/>',
panelClose: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18M16 9l-3 3 3 3"/>',
panelOpen: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18M14 9l3 3-3 3"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.42 1.42M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.42"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
  refresh: '<path d="M20 11a8.1 8.1 0 0 0-15.5-2M4 4v5h5M4 13a8.1 8.1 0 0 0 15.5 2M20 20v-5h-5"/>',
  more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  edit: '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"/>',
  trash: '<path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v5M14 11v5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  upload: '<path d="M12 3v12M7 8l5-5 5 5M5 21h14"/>',
  download: '<path d="M12 3v12M7 10l5 5 5-5M5 21h14"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  play: '<path d="m8 5 11 7-11 7Z"/>',
  copy: '<rect width="13" height="13" x="9" y="9" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  link: '<path d="M10 13a5 5 0 0 0 7.5.5l2-2a5 5 0 0 0-7-7l-1.1 1.1M14 11a5 5 0 0 0-7.5-.5l-2 2a5 5 0 0 0 7 7l1.1-1.1"/>',
  key: '<circle cx="7.5" cy="15.5" r="4.5"/><path d="m10.8 12.2 8.7-8.7M15 8l2 2M17 6l2 2"/>',
  bolt: '<path d="m13 2-9 12h8l-1 8 9-12h-8l1-8Z"/>',
  sparkles: '<path d="m12 3-1.2 4.8L6 9l4.8 1.2L12 15l1.2-4.8L18 9l-4.8-1.2L12 3ZM5 16l-.7 2.3L2 19l2.3.7L5 22l.7-2.3L8 19l-2.3-.7L5 16ZM19 13l-.7 2.3-2.3.7 2.3.7L19 19l.7-2.3L22 16l-2.3-.7L19 13Z"/>',
  externalLink: '<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  lock: '<rect width="16" height="12" x="4" y="10" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
  logout: '<path d="M10 17l5-5-5-5M15 12H3M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/>',
};
const adminNavigation = [
{ key: "overview" },
{ key: "opsAdmin" },
{ key: "users" },
{ key: "groups" },
{ group: "channel-management", label: "频道管理", icon: "radio", children: [
  { key: "channelsAdmin" }, { key: "monitorAdmin" },
] },
{ key: "plans", label: "订阅管理" },
{ key: "accounts" },
{ key: "content" },
{ key: "proxies" },
{ group: "security-audit", label: "安全审计", icon: "shield", children: [
  { key: "riskAdmin" }, { key: "promptAuditAdmin" },
] },
{ key: "redeemAdmin" },
{ key: "ordersAdmin" },
{ key: "usage" },
{ key: "audit" },
{ key: "settings" },
];
const selfNavigation = [
{ key: "keys" },
{ key: "batchImages" },
{ group: "api-resources", label: "API 资源", icon: "box", children: [
  { key: "models" }, { key: "channels" }, { key: "monitor" },
] },
{ key: "subscriptions" },
{ key: "redeem" },
{ key: "profile" },
];
const userNavigation = [
{ key: "overview" },
...selfNavigation.slice(0, 2),
{ key: "usage" },
...selfNavigation.slice(2),
];
const expandedNavGroups = new Set();
let mobileNavigationOpen = false;
let sidebarCollapsed = localStorage.getItem("mini_sidebar_collapsed") === "true";
let currentKeys = [];
let currentKeyOwners = [];
let selectedKeyIds = new Set();
let currentAccounts = [];
let selectedAccountIds = new Set();
let accountFilters = { search: "", platform: "", kind: "", status: "", group: "" };
const accountOptionalColumns = [
  { key: "id", label: "ID" },
  { key: "platform_type", label: "平台 / 类型" },
  { key: "capacity", label: "容量" },
  { key: "status", label: "状态" },
  { key: "schedulable", label: "可调度" },
  { key: "today_stats", label: "今日统计" },
  { key: "groups", label: "分组" },
  { key: "usage", label: "用量窗口" },
  { key: "proxy", label: "代理" },
  { key: "priority", label: "优先级" },
  { key: "last_used_at", label: "最近使用" },
  { key: "created_at", label: "创建时间" },
  { key: "expires_at", label: "过期时间" },
  { key: "notes", label: "备注" },
];
const accountColumnStorageKey = "mini_account_hidden_columns";
const accountColumnVersionKey = "mini_account_hidden_columns_version";
const accountColumnVersion = "original-account-layout-v1";
let hiddenAccountColumns = storedAccountHiddenColumns();
const accountSortStorageKey = "mini_account_table_sort";
let accountSort = storedAccountSort();
let accountPage = 1;
let accountPageSize = [10, 20, 50, 100].includes(Number(localStorage.getItem("mini_account_page_size"))) ? Number(localStorage.getItem("mini_account_page_size")) : 20;
const accountAutoRefreshOptions = [5, 10, 15, 30];
const storedAccountAutoRefresh = readStoredAccountAutoRefresh();
let accountAutoRefreshEnabled = storedAccountAutoRefresh.enabled;
let accountAutoRefreshSeconds = storedAccountAutoRefresh.interval_seconds;
let accountAutoRefreshTimer = null;
let accountAutoRefreshDeadline = 0;
let activeUpstreamAccountMenu = null;
let activeUpstreamAccountMenuTrigger = null;
let currentProxies = [];
let currentTlsProfiles = [];
let currentMonitors = [];
let currentChannels = [];
let selectedProxyIds = new Set();
let usagePage = 1;
let usageFilters = {};
let currentGroups = [];
let currentPlans = [];
let currentSubscriptions = [];
let currentRedeemCodes = [];
let auditPage = 1;
let auditFilters = {};
let riskLogPage = 1;
let riskLogFilters = {};
let currentRiskLogs = [];
let currentRiskGroups = [];
let promptEventPage = 1;
let promptEventFilters = {};
let currentPromptConfig = null;
let currentPromptRuntime = null;
let currentPromptEndpoints = [];
let currentPromptEvents = [];
let currentPromptGroups = [];
let selectedPromptEventIds = new Set();
const featureScripts = new Map();
document.addEventListener("DOMContentLoaded", init);
window.addEventListener("hashchange", navigate);
document.addEventListener("click", event => {
  const tools = document.querySelector("#account-tools-dropdown");
  const toolsMenu = document.querySelector("#account-tools-menu");
  if (tools && !tools.contains(event.target) && !toolsMenu?.contains(event.target)) closeAccountToolsMenu();
  const autoRefresh = document.querySelector("#account-auto-refresh-dropdown");
  if (autoRefresh && !autoRefresh.contains(event.target)) closeAccountAutoRefreshMenu();
  if (!activeUpstreamAccountMenu) return;
  if (activeUpstreamAccountMenu.contains(event.target) || event.target.closest("[data-account-menu]")) return;
  closeUpstreamAccountMenu();
});
document.addEventListener("keydown", event => {
  if (event.key !== "Escape") return;
  closeAccountToolsMenu();
  closeAccountAutoRefreshMenu();
  closeUpstreamAccountMenu();
});
function loadFeatureScript(name) {
if (featureScripts.has(name)) return featureScripts.get(name);
const promise = new Promise((resolve, reject) => {
  const script = document.createElement("script");
  script.src = `/${name}.js`;
  script.async = true;
  script.addEventListener("load", resolve, { once: true });
  script.addEventListener("error", () => reject(new Error(`无法载入 ${name} 页面资源`)), { once: true });
  document.head.append(script);
});
featureScripts.set(name, promise);
return promise;
}
async function init() {
try {
  const settings = await api("/api/public/settings");
  state.siteName = settings.data.site_name || state.siteName;
  state.siteSubtitle = settings.data.site_subtitle || state.siteSubtitle;
  state.siteLogo = settings.data.site_logo || state.siteLogo;
  state.version = settings.data.version || "";
  state.defaultTheme = normalizeTheme(settings.data.default_theme);
  applyTheme(localStorage.getItem(THEME_STORAGE_KEY) || state.defaultTheme);
  state.contactInfo = settings.data.contact_info || "";
  state.docUrl = settings.data.doc_url || "";
  state.homeContent = settings.data.home_content || "";
  state.homeContentHtml = settings.data.home_content_html || "";
  state.homeContentUrl = settings.data.home_content_url || null;
  state.registrationEnabled = Boolean(settings.data.registration_enabled);
  state.emailVerificationEnabled = Boolean(settings.data.email_verification_enabled);
  state.passwordResetEnabled = Boolean(settings.data.password_reset_enabled);
  state.mailConfigured = Boolean(settings.data.mail_configured);
  state.turnstileEnabled = Boolean(settings.data.turnstile_enabled && settings.data.turnstile_site_key);
  state.turnstileSiteKey = settings.data.turnstile_site_key || "";
  document.title = state.siteName;
} catch (_) {}
await navigate();
}
async function navigate() {
const routeName = currentRouteName();
if (routeName === "email-verify" && sessionStorage.getItem("mini_pending_registration")) {
  renderRegistrationEmailVerification();
  return;
}
if (routeName === "setup") {
  await loadFeatureScript("setup");
  await window.Sub2MiniSetup.render();
  return;
}
if (routeName === "key-usage") {
  renderPublicKeyUsage();
  return;
}
if (routeName === "home") {
  await renderPublicHome();
  return;
}
if (routeName.startsWith("page/")) {
  await renderPublicPage(routeName.slice(5));
  return;
}
if (["register", "forgot-password", "reset-password"].includes(routeName)) {
  if (!state.user) {
    try {
      const result = await api("/api/auth/me");
      applyIdentity(result.data);
    } catch (_) {}
  }
  if (state.user) {
    location.hash = "#/overview";
    if (!document.querySelector(".app-shell")) renderShell();
    await renderRoute();
    return;
  }
  if (routeName === "register") renderRegister();
  if (routeName === "forgot-password") renderForgotPassword();
  if (routeName === "reset-password") renderResetPassword();
  return;
}
if (state.user) {
  if (!document.querySelector(".app-shell")) renderShell();
  await renderRoute();
  return;
}
try {
  const result = await api("/api/auth/me");
  applyIdentity(result.data);
  renderShell();
  await renderRoute();
} catch (_) {
  renderLogin();
}
}
function currentRouteName() {
return location.hash.replace(/^#\/?/, "").split("?", 1)[0];
}
function siteLogo() {
return escapeHtml(state.siteLogo || "/logo.svg");
}
async function api(path, options = {}) {
const headers = new Headers(options.headers || {});
if (options.body && !(options.body instanceof FormData)) headers.set("Content-Type", "application/json");
if (state.csrf && options.method && !["GET", "HEAD"].includes(options.method)) {
  headers.set("X-CSRF-Token", state.csrf);
}
const response = await fetch(`${API}${path}`, { ...options, headers, credentials: "include" });
if (response.status === 204) return null;
const data = await response.json().catch(() => ({}));
if (!response.ok) {
  const error = new Error(data.error?.message || `请求失败 (${response.status})`);
  error.status = response.status;
  error.code = data.error?.code;
  throw error;
}
return data;
}
function setCsrf(value) {
state.csrf = value;
sessionStorage.setItem("mini_csrf", value);
}
function applyIdentity(data) {
state.user = data.username;
state.displayName = data.display_name || data.username;
state.role = data.role;
sessionStorage.removeItem("mini_pending_registration");
setCsrf(data.csrf_token);
}
function activeRoutes() {
return Object.entries(routes).filter(([key, route]) => {
  if (route.adminOnly && state.role !== "admin") return false;
  return true;
});
}
function roleApiBase() {
return state.role === "admin" ? "/api/admin" : "/api/user";
}
function renderLogin(message = "") {
state.user = null;
state.displayName = null;
state.role = null;
app.innerHTML = `
  <main class="login-screen">
    <section class="login-layout">
      <div class="login-brand">
        <div>
          <img src="${siteLogo()}" alt="">
          <h1>${escapeHtml(state.siteName)}</h1>
          <p>${escapeHtml(state.siteSubtitle)}</p>
        </div>
        <small>SQLite / Axum / 单进程</small>
      </div>
      <div class="login-form-wrap">
        <h2>账户登录</h2>
        <p>进入 API 网关控制台</p>
        <form id="login-form">
          <div class="field">
            <label for="username">用户名或邮箱</label>
            <input id="username" name="username" autocomplete="username" required autofocus>
          </div>
          <div class="field">
            <label for="password">密码</label>
            <input id="password" name="password" type="password" autocomplete="current-password" required>
          </div>
          ${turnstileField()}
          <button class="button" type="submit">登录</button>
          <p id="login-error" class="form-error">${escapeHtml(message)}</p>
        </form>
        <div class="login-links"><a class="text-link" href="#/home">公共首页</a><a class="text-link" href="#/key-usage">查询 API Key 用量</a>${state.registrationEnabled ? '<a class="text-link" href="#/register">注册账户</a>' : ""}${state.passwordResetEnabled && state.mailConfigured ? '<a class="text-link" href="#/forgot-password">忘记密码</a>' : ""}</div>
      </div>
    </section>
  </main>`;
document.querySelector("#login-form").addEventListener("submit", handleLogin);
mountTurnstile(document.querySelector("#login-form"));
}
function turnstileField() { return state.turnstileEnabled ? '<input name="turnstile_token" type="hidden"><div class="turnstile-slot" data-turnstile-slot></div>' : ""; }
function mountTurnstile(form) {
if (!state.turnstileEnabled || !form) return;
loadFeatureScript("turnstile").then(() => window.Sub2MiniTurnstile.mount(form, state.turnstileSiteKey)).catch(error => { const output = form.querySelector(".form-error"); if (output) output.textContent = error.message; });
}
function resetTurnstile(form) { if (state.turnstileEnabled) window.Sub2MiniTurnstile?.reset(form); }
async function handleLogin(event) {
event.preventDefault();
const form = event.currentTarget;
const button = form.querySelector("button");
const error = form.querySelector("#login-error");
button.disabled = true;
error.textContent = "";
try {
  const result = await api("/api/auth/login", {
    method: "POST",
    body: JSON.stringify(Object.fromEntries(new FormData(form))),
  });
  if (result.data.requires_2fa) {
    openTotpLogin(result.data.temp_token, result.data.user_email_masked);
    return;
  }
  applyIdentity(result.data);
  location.hash = "#/overview";
  renderShell();
  await renderRoute();
} catch (requestError) {
  error.textContent = requestError.message;
  resetTurnstile(form);
} finally {
  button.disabled = false;
}
}
function openTotpLogin(tempToken, identifier) {
openModal("双因素验证", `<form id="totp-login-form"><p class="field-hint">${escapeHtml(identifier || "账户")}</p><div class="field"><label for="totp-login-code">动态码或恢复码</label><input id="totp-login-code" name="totp_code" autocomplete="one-time-code" required autofocus></div><p class="form-error" id="totp-login-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="submit-totp-login">验证</button>`);
modal.querySelector("#submit-totp-login").addEventListener("click", async event => {
  const form = modal.querySelector("#totp-login-form");
  if (!form.reportValidity()) return;
  event.currentTarget.disabled = true;
  try {
    const result = await api("/api/auth/login/2fa", { method: "POST", body: JSON.stringify({ temp_token: tempToken, totp_code: form.elements.totp_code.value }) });
    closeModal(); applyIdentity(result.data); location.hash = "#/overview"; renderShell(); await renderRoute();
  } catch (error) { modal.querySelector("#totp-login-error").textContent = error.message; event.currentTarget.disabled = false; }
});
}
function renderAuthScreen(title, subtitle, body, links = "") {
state.user = null;
state.displayName = null;
state.role = null;
app.innerHTML = `
  <main class="login-screen">
    <section class="login-layout">
      <div class="login-brand"><div><img src="${siteLogo()}" alt=""><h1>${escapeHtml(state.siteName)}</h1><p>${escapeHtml(state.siteSubtitle)}</p></div><small>SQLite / Axum / 单进程</small></div>
      <div class="login-form-wrap auth-form-wrap"><h2>${escapeHtml(title)}</h2><p>${escapeHtml(subtitle)}</p>${body}<div class="login-links">${links}</div></div>
    </section>
  </main>`;
}
function renderRegister() {
if (!state.registrationEnabled) {
  renderAuthScreen("注册未开放", "当前实例仅允许管理员创建用户", `<p class="auth-notice">需要账户时请联系管理员。</p>`, '<a class="text-link" href="#/overview">返回登录</a>');
  return;
}
renderAuthScreen("注册账户", `创建 ${state.siteName} 账户`, `
  <form id="register-form">
    <div class="field"><label for="register-email">邮箱</label><input id="register-email" name="email" type="email" autocomplete="email" maxlength="254" required autofocus></div>
    <div class="field"><label for="register-password">密码</label><input id="register-password" name="password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required><span class="field-hint">8 至 128 个字符</span></div>
    <div class="field"><label for="register-confirm">确认密码</label><input id="register-confirm" name="confirm_password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required></div>
    ${turnstileField()}
    <button class="button auth-submit" type="submit">${state.emailVerificationEnabled ? "继续验证邮箱" : "创建账户"}</button><p class="form-error" id="register-error"></p>
  </form>`, '<a class="text-link" href="#/overview">已有账户，返回登录</a><a class="text-link" href="#/home">公共首页</a>');
document.querySelector("#register-form").addEventListener("submit", handleRegister);
mountTurnstile(document.querySelector("#register-form"));
}
async function handleRegister(event) {
event.preventDefault();
const form = event.currentTarget;
const values = Object.fromEntries(new FormData(form));
const error = form.querySelector("#register-error");
const button = form.querySelector("button[type=submit]");
error.textContent = "";
if (values.password !== values.confirm_password) {
  error.textContent = "两次输入的密码不一致";
  return;
}
delete values.confirm_password;
button.disabled = true;
try {
  if (state.emailVerificationEnabled) {
    const sent = await api("/api/auth/send-verification-code", { method: "POST", body: JSON.stringify({ email: values.email }) });
    sessionStorage.setItem("mini_pending_registration", JSON.stringify({ ...values, verification_sent_at: Date.now(), countdown: sent.data.countdown }));
    location.hash = "#/email-verify";
    return;
  }
  const result = await api("/api/auth/register", { method: "POST", body: JSON.stringify(values) });
  applyIdentity(result.data);
  location.hash = "#/overview";
  renderShell();
  await renderRoute();
  toast("账户已创建");
} catch (requestError) { error.textContent = requestError.message; resetTurnstile(form); }
finally { button.disabled = false; }
}
function renderRegistrationEmailVerification() {
let registration;
try { registration = JSON.parse(sessionStorage.getItem("mini_pending_registration") || "null"); }
catch (_) { registration = null; }
if (!registration?.email || !registration?.password) {
  sessionStorage.removeItem("mini_pending_registration");
  renderAuthScreen("验证信息已失效", "请重新填写注册信息", '<p class="auth-notice">待注册资料只保留在当前浏览器会话中。</p>', '<a class="text-link" href="#/register">返回注册</a>');
  return;
}
renderAuthScreen("验证邮箱", "输入发送到注册邮箱的一次性验证码", `
  <form id="registration-verification-form">
    <div class="field"><label for="registration-email">注册邮箱</label><input id="registration-email" value="${escapeHtml(registration.email)}" disabled></div>
    <div class="field"><label for="registration-code">邮箱验证码</label><input id="registration-code" name="verify_code" maxlength="16" autocomplete="one-time-code" required autofocus></div>
    ${turnstileField()}
    <div class="inline-field"><button class="button auth-submit" type="submit">验证并创建账户</button><button class="button secondary" id="resend-registration-code" type="button">重新发送</button></div>
    <span class="field-hint" id="registration-verification-hint">验证码 10 分钟内有效</span><p class="form-error" id="registration-verification-error"></p>
  </form>`, '<a class="text-link" id="cancel-registration-verification" href="#/register">返回修改注册信息</a>');
const form = document.querySelector("#registration-verification-form");
const resend = document.querySelector("#resend-registration-code");
const hint = document.querySelector("#registration-verification-hint");
mountTurnstile(form);
const updateCountdown = () => {
  const remaining = Math.max(0, Number(registration.countdown || 60) - Math.floor((Date.now() - Number(registration.verification_sent_at || 0)) / 1000));
  resend.disabled = remaining > 0;
  hint.textContent = remaining > 0 ? `验证码已发送，${remaining} 秒后可重新发送` : "没有收到邮件时可以重新发送";
  if (remaining > 0 && resend.isConnected) setTimeout(updateCountdown, 1000);
};
updateCountdown();
resend.addEventListener("click", async () => {
  resend.disabled = true;
  try {
    const sent = await api("/api/auth/send-verification-code", { method: "POST", body: JSON.stringify({ email: registration.email, turnstile_token: window.Sub2MiniTurnstile?.token(form) || "" }) });
    registration.verification_sent_at = Date.now(); registration.countdown = sent.data.countdown;
    sessionStorage.setItem("mini_pending_registration", JSON.stringify(registration));
    updateCountdown();
  } catch (error) { hint.textContent = error.message; resend.disabled = false; resetTurnstile(form); }
});
document.querySelector("#cancel-registration-verification").addEventListener("click", () => sessionStorage.removeItem("mini_pending_registration"));
form.addEventListener("submit", async event => {
  event.preventDefault();
  const button = form.querySelector("button[type=submit]");
  const error = form.querySelector("#registration-verification-error");
  button.disabled = true; error.textContent = "";
  const payload = { ...registration, verify_code: form.elements.verify_code.value };
  delete payload.verification_sent_at; delete payload.countdown;
  try {
    const result = await api("/api/auth/register", { method: "POST", body: JSON.stringify(payload) });
    applyIdentity(result.data); location.hash = "#/overview"; renderShell(); await renderRoute(); toast("账户已创建");
  } catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
});
}
function renderForgotPassword() {
if (!state.passwordResetEnabled || !state.mailConfigured) {
  renderAuthScreen("密码找回不可用", "管理员尚未配置邮件投递", `<p class="auth-notice">请联系管理员重置账户密码。</p>`, '<a class="text-link" href="#/overview">返回登录</a>');
  return;
}
renderAuthScreen("找回密码", "重置链接将发送到注册邮箱", `
  <form id="forgot-password-form">
    <div class="field"><label for="forgot-email">邮箱</label><input id="forgot-email" name="email" type="email" autocomplete="email" required autofocus></div>
    ${turnstileField()}
    <button class="button auth-submit" type="submit">发送重置链接</button><p class="form-error" id="forgot-error"></p>
  </form>`, '<a class="text-link" href="#/overview">返回登录</a>');
document.querySelector("#forgot-password-form").addEventListener("submit", handleForgotPassword);
mountTurnstile(document.querySelector("#forgot-password-form"));
}
async function handleForgotPassword(event) {
event.preventDefault();
const form = event.currentTarget;
const button = form.querySelector("button");
const error = form.querySelector("#forgot-error");
button.disabled = true; error.textContent = "";
try {
  await api("/api/auth/forgot-password", { method: "POST", body: JSON.stringify(Object.fromEntries(new FormData(form))) });
  form.innerHTML = '<p class="auth-success">如果邮箱已注册，重置链接会很快送达。</p>';
} catch (requestError) { error.textContent = requestError.message; resetTurnstile(form); }
finally { button.disabled = false; }
}
function renderResetPassword() {
const params = new URLSearchParams(location.hash.split("?", 2)[1] || "");
const email = params.get("email") || "";
const token = params.get("token") || "";
if (!state.passwordResetEnabled) {
  renderAuthScreen("密码重置已关闭", "当前实例不接受密码重置请求", "", '<a class="text-link" href="#/overview">返回登录</a>');
  return;
}
renderAuthScreen("设置新密码", "有效链接仅可使用一次", `
  <form id="reset-password-form">
    <div class="field"><label for="reset-email">邮箱</label><input id="reset-email" name="email" type="email" value="${escapeHtml(email)}" autocomplete="email" required ${email ? "readonly" : ""}></div>
    <div class="field"><label for="reset-token">重置令牌</label><input id="reset-token" name="token" value="${escapeHtml(token)}" autocomplete="off" required ${token ? "readonly" : ""}></div>
    <div class="field"><label for="reset-new-password">新密码</label><input id="reset-new-password" name="new_password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required autofocus></div>
    <div class="field"><label for="reset-confirm-password">确认新密码</label><input id="reset-confirm-password" name="confirm_password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required></div>
    <button class="button auth-submit" type="submit">重置密码</button><p class="form-error" id="reset-error"></p>
  </form>`, '<a class="text-link" href="#/overview">返回登录</a>');
document.querySelector("#reset-password-form").addEventListener("submit", handleResetPassword);
}
async function handleResetPassword(event) {
event.preventDefault();
const form = event.currentTarget;
const values = Object.fromEntries(new FormData(form));
const error = form.querySelector("#reset-error");
if (values.new_password !== values.confirm_password) {
  error.textContent = "两次输入的密码不一致";
  return;
}
delete values.confirm_password;
const button = form.querySelector("button");
button.disabled = true; error.textContent = "";
try {
  await api("/api/auth/reset-password", { method: "POST", body: JSON.stringify(values) });
  location.hash = "#/overview";
  renderLogin("密码已重置，请使用新密码登录");
} catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
function resolveNavigationItems(items, allowed) {
return items.flatMap(item => {
  if (item.key) {
    const route = allowed.get(item.key);
    return route ? [{ ...item, label: item.label || route.label }] : [];
  }
  const children = resolveNavigationItems(item.children || [], allowed);
  return children.length ? [{ ...item, children }] : [];
});
}
function navigationSections(allowed) {
if (state.role === "admin") {
  return [
    { id: "admin", label: "", items: resolveNavigationItems(adminNavigation, allowed) },
    { id: "self", label: "我的账户", items: resolveNavigationItems(selfNavigation, allowed) },
  ];
}
return [{ id: "user", label: "控制台", items: resolveNavigationItems(userNavigation, allowed) }];
}
function navigationGroupActive(item, routeName) {
return item.children?.some(child => child.key === routeName) || false;
}
function navigationGroupExpanded(item, routeName) {
return expandedNavGroups.has(item.group) || navigationGroupActive(item, routeName);
}
function appIcon(name, className = "") {
const shape = iconShapes[name] || iconShapes.file;
return `<svg class="app-icon ${className}" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${shape}</svg>`;
}
function navigationIcon(item) {
return appIcon(item.icon || routeIcons[item.key] || "file", "nav-icon");
}
function renderNavigationItems(items, routeName) {
return items.map(item => {
  if (item.key) {
    const active = item.key === routeName;
    return `<a href="#/${item.key}" data-route="${item.key}" title="${escapeHtml(item.label)}" ${active ? 'class="active" aria-current="page"' : ""}>${navigationIcon(item)}<span class="nav-label">${escapeHtml(item.label)}</span></a>`;
  }
  const active = navigationGroupActive(item, routeName);
  const expanded = navigationGroupExpanded(item, routeName);
  return `<div class="nav-group ${active ? "active" : ""}" data-nav-group="${escapeHtml(item.group)}">
    <button class="nav-group-toggle" type="button" data-nav-group-toggle="${escapeHtml(item.group)}" aria-expanded="${expanded}" title="${escapeHtml(item.label)}">${navigationIcon(item)}<span class="nav-label">${escapeHtml(item.label)}</span>${appIcon("chevronDown", "nav-chevron")}</button>
    <div class="nav-children" data-nav-group-children="${escapeHtml(item.group)}" ${expanded ? "" : "hidden"}>${renderNavigationItems(item.children, routeName)}</div>
  </div>`;
}).join("");
}
function setMobileNavigation(open) {
mobileNavigationOpen = Boolean(open);
const sidebar = document.querySelector(".sidebar");
const overlay = document.querySelector("#mobile-nav-overlay");
const toggle = document.querySelector("#mobile-nav-toggle");
sidebar?.classList.toggle("mobile-open", mobileNavigationOpen);
if (overlay) overlay.hidden = !mobileNavigationOpen;
toggle?.setAttribute("aria-expanded", String(mobileNavigationOpen));
}
function toggleNavigationGroup(event) {
if (sidebarCollapsed) return;
const groupId = event.currentTarget.dataset.navGroupToggle;
const children = document.querySelector(`[data-nav-group-children="${groupId}"]`);
const expanded = event.currentTarget.getAttribute("aria-expanded") === "true";
if (expanded) expandedNavGroups.delete(groupId); else expandedNavGroups.add(groupId);
event.currentTarget.setAttribute("aria-expanded", String(!expanded));
if (children) children.hidden = expanded;
}
function identityInitials() {
const value = String(state.displayName || state.user || "U").trim();
return Array.from(value).slice(0, 2).join("").toUpperCase();
}
function normalizeTheme(theme) {
return theme === "dark" ? "dark" : "light";
}
function applyTheme(theme, persist = false) {
state.theme = normalizeTheme(theme);
document.documentElement.dataset.theme = state.theme;
document.documentElement.style.colorScheme = state.theme;
document.querySelector('meta[name="theme-color"]')?.setAttribute("content", state.theme === "dark" ? "#0f172a" : "#f8fafc");
if (persist) localStorage.setItem(THEME_STORAGE_KEY, state.theme);
const button = document.querySelector("#theme-toggle");
if (button) {
  const target = state.theme === "dark" ? "亮色模式" : "暗色模式";
  button.innerHTML = `${appIcon(state.theme === "dark" ? "sun" : "moon")}<span>${target}</span>`;
  button.setAttribute("title", `切换到${target}`);
  button.setAttribute("aria-pressed", String(state.theme === "dark"));
}
}
function toggleTheme() {
applyTheme(state.theme === "dark" ? "light" : "dark", true);
}
function setSidebarCollapsed(collapsed) {
sidebarCollapsed = Boolean(collapsed);
localStorage.setItem("mini_sidebar_collapsed", String(sidebarCollapsed));
const shell = document.querySelector(".app-shell");
const button = document.querySelector("#sidebar-collapse");
shell?.classList.toggle("sidebar-collapsed", sidebarCollapsed);
button?.setAttribute("aria-expanded", String(!sidebarCollapsed));
button?.setAttribute("title", sidebarCollapsed ? "展开菜单" : "折叠菜单");
if (button) button.innerHTML = `${appIcon(sidebarCollapsed ? "panelOpen" : "panelClose")}<span>${sidebarCollapsed ? "展开菜单" : "折叠菜单"}</span>`;
}
function setAccountMenu(open) {
const menu = document.querySelector("#account-dropdown");
const toggle = document.querySelector("#account-toggle");
if (menu) menu.hidden = !open;
toggle?.setAttribute("aria-expanded", String(Boolean(open)));
}
function syncPageChrome(route) {
const header = document.querySelector("#page .page-header");
const title = header?.querySelector("h1")?.textContent?.trim() || route.label;
const description = header?.querySelector("p")?.textContent?.trim() || "";
const topTitle = document.querySelector("#topbar-title");
const topDescription = document.querySelector("#topbar-description");
if (topTitle) topTitle.textContent = title;
if (topDescription) {
  topDescription.textContent = description;
  topDescription.hidden = !description;
}
document.title = `${title} · ${state.siteName}`;
if (header) {
  header.classList.add("page-header-synced");
  header.classList.toggle("topbar-only", !header.querySelector(".actions"));
}
}
function renderShell() {
const allowed = new Map(activeRoutes().filter(([, route]) => !route.hidden));
const routeName = currentRouteName();
const sections = navigationSections(allowed);
const initialRoute = allowed.get(routeName) || routes.overview;
const roleLabel = state.role === "admin" ? "管理员" : "用户";
app.innerHTML = `
  <div id="mobile-nav-overlay" class="mobile-nav-overlay" hidden></div>
  <div class="app-shell ${sidebarCollapsed ? "sidebar-collapsed" : ""}">
    <aside class="sidebar">
      <div class="sidebar-header"><a class="brand" href="#/overview" title="${escapeHtml(state.siteName)}"><img src="${siteLogo()}" alt=""><span>${escapeHtml(state.siteName)}<small>${state.version ? `v${escapeHtml(state.version)} · ` : ""}MINI</small></span></a></div>
      <nav class="nav-list" aria-label="主导航">
        ${sections.map(section => `<section class="nav-section" data-nav-section="${section.id}">${section.label ? `<h2>${escapeHtml(section.label)}</h2>` : ""}${renderNavigationItems(section.items, routeName)}</section>`).join("")}
      </nav>
      <div class="sidebar-footer"><button id="theme-toggle" class="sidebar-control theme-control" type="button" aria-pressed="${state.theme === "dark"}" title="切换到${state.theme === "dark" ? "亮色模式" : "暗色模式"}">${appIcon(state.theme === "dark" ? "sun" : "moon")}<span>${state.theme === "dark" ? "亮色模式" : "暗色模式"}</span></button><button id="sidebar-collapse" class="sidebar-control collapse-control" type="button" aria-expanded="${!sidebarCollapsed}" title="${sidebarCollapsed ? "展开菜单" : "折叠菜单"}">${appIcon(sidebarCollapsed ? "panelOpen" : "panelClose")}<span>${sidebarCollapsed ? "展开菜单" : "折叠菜单"}</span></button></div>
    </aside>
    <div class="main-area">
      <header class="app-topbar">
        <div class="topbar-mobile-brand"><button id="mobile-nav-toggle" class="icon-button" type="button" aria-label="打开主导航" aria-expanded="false">${appIcon("menu")}</button><a href="#/overview"><img src="${siteLogo()}" alt=""><strong>${escapeHtml(state.siteName)}</strong></a></div>
        <div class="topbar-page"><h1 id="topbar-title">${escapeHtml(initialRoute.label)}</h1><p id="topbar-description" hidden></p></div>
        <div class="account-menu">
          <button id="account-toggle" class="account-toggle" type="button" aria-label="用户菜单" aria-expanded="false"><span class="account-avatar">${escapeHtml(identityInitials())}</span><span class="account-copy"><strong>${escapeHtml(state.displayName || state.user || "")}</strong><small>${roleLabel}</small></span>${appIcon("chevronDown", "account-chevron")}</button>
          <div id="account-dropdown" class="account-dropdown" hidden><div class="account-dropdown-head"><strong>${escapeHtml(state.displayName || state.user || "")}</strong><span>${roleLabel}</span></div><a href="#/profile">${appIcon("user")}<span>个人资料</span></a><a href="#/keys">${appIcon("key")}<span>API Key</span></a><button id="logout-button" type="button">${appIcon("logout")}<span>退出登录</span></button></div>
        </div>
      </header>
      <main class="workspace"><div id="page" class="page"></div></main>
    </div>
  </div>`;
document.querySelector("#logout-button")?.addEventListener("click", logout);
document.querySelector("#mobile-nav-toggle")?.addEventListener("click", () => setMobileNavigation(!mobileNavigationOpen));
document.querySelector("#mobile-nav-overlay")?.addEventListener("click", () => setMobileNavigation(false));
document.querySelector("#sidebar-collapse")?.addEventListener("click", () => setSidebarCollapsed(!sidebarCollapsed));
document.querySelector("#theme-toggle")?.addEventListener("click", toggleTheme);
document.querySelector("#account-toggle")?.addEventListener("click", event => {
  event.stopPropagation();
  const menu = document.querySelector("#account-dropdown");
  setAccountMenu(Boolean(menu?.hidden));
});
document.querySelector(".workspace")?.addEventListener("click", () => setAccountMenu(false));
document.querySelectorAll("[data-nav-group-toggle]").forEach(button => button.addEventListener("click", toggleNavigationGroup));
document.querySelectorAll(".nav-list a, .account-dropdown a").forEach(link => link.addEventListener("click", () => { setMobileNavigation(false); setAccountMenu(false); }));
setMobileNavigation(false);
}
async function renderRoute() {
const hash = location.hash.replace(/^#\/?/, "");
const [routeName, query = ""] = hash.split("?");
const allowed = new Map(activeRoutes());
const route = allowed.get(routeName) || routes.overview;
const activeRouteName = allowed.has(routeName) ? routeName : "overview";
closeUpstreamAccountMenu();
stopAccountAutoRefresh();
document.querySelectorAll("[data-route]").forEach(link => {
  const active = link.dataset.route === activeRouteName;
  link.classList.toggle("active", active);
  if (active) link.setAttribute("aria-current", "page"); else link.removeAttribute("aria-current");
});
document.querySelectorAll("[data-nav-group]").forEach(group => {
  const childActive = Boolean(group.querySelector(`[data-route="${activeRouteName}"]`));
  group.classList.toggle("active", childActive);
  if (childActive) {
    const toggle = group.querySelector("[data-nav-group-toggle]");
    const children = group.querySelector("[data-nav-group-children]");
    toggle?.setAttribute("aria-expanded", "true");
    if (children) children.hidden = false;
  }
});
const page = document.querySelector("#page");
if (!page) return;
page.innerHTML = `<div class="boot-screen"><p>正在载入</p></div>`;
try {
  await route.render(page);
  syncPageChrome(route);
  if (state.user && routeName !== "announcements") {
    try {
      await loadFeatureScript("engagement");
      await window.Sub2MiniEngagement.maybeShowPopup();
    } catch (_) {}
  }
  const params = new URLSearchParams(query);
  if (params.get("oauth") === "success") toast("OAuth 账号已添加");
  if (params.get("oauth") === "reauthorized") toast("OAuth 账号已重新授权");
  if (params.get("oauth") === "error") toast("OAuth 授权失败", true);
} catch (error) {
  if (error.status === 401) return renderLogin("会话已过期，请重新登录");
  page.innerHTML = emptyState("载入失败", error.message, "重新载入", "reload-route");
  syncPageChrome(route);
  document.querySelector("#reload-route")?.addEventListener("click", renderRoute);
}
}
async function renderOverview(page) {
await loadFeatureScript("dashboard");
return window.Sub2MiniDashboard.render(page);
}
async function renderUsers(page) {
await loadFeatureScript("users");
return window.Sub2MiniUsers.render(page);
}
async function renderSubscriptionsFeature(page) {
await loadFeatureScript("subscriptions");
return window.Sub2MiniSubscriptions.renderUser(page);
}
async function renderPlanAdminFeature(page) {
await loadFeatureScript("subscriptions");
return window.Sub2MiniSubscriptions.renderAdmin(page);
}
async function renderChannelAdminFeature(page) {
await loadFeatureScript("channels");
return window.Sub2MiniChannels.renderAdmin(page);
}
async function renderOpsAdmin(page) {
await loadFeatureScript("ops");
return window.Sub2MiniOps.render(page);
}
async function renderBatchImages(page) {
await loadFeatureScript("batch-images");
return window.Sub2MiniBatchImages.render(page);
}
async function renderAccounts(page) {
await loadFeatureScript("accounts-tools");
await loadFeatureScript("account-schedules");
const [result, proxies, groups, tlsProfiles] = await Promise.all([
  api("/api/admin/accounts"),
  api("/api/admin/proxies"),
  api("/api/admin/groups"),
  api("/api/admin/tls-fingerprint-profiles"),
]);
currentAccounts = result.data;
currentProxies = proxies.data;
currentGroups = groups.data;
currentTlsProfiles = tlsProfiles.data;
selectedAccountIds = new Set([...selectedAccountIds].filter(id => currentAccounts.some(account => account.id === id)));
page.innerHTML = `
  ${pageHeader("账号管理", `${result.data.length} 个账号 · Claude 与 OpenAI 上游连接及调度`)}
  <section class="account-page-toolbar" aria-label="账号筛选与操作">
    <div class="account-filters">
      <label class="account-search">${appIcon("search")}<input id="account-search" type="search" value="${escapeHtml(accountFilters.search)}" placeholder="搜索账号..." aria-label="搜索账号"></label>
      <select id="account-platform-filter" aria-label="账号平台"><option value="">全部平台</option><option value="anthropic" ${accountFilters.platform === "anthropic" ? "selected" : ""}>Anthropic</option><option value="openai" ${accountFilters.platform === "openai" ? "selected" : ""}>OpenAI</option></select>
      <select id="account-kind-filter" aria-label="账号类型"><option value="">全部类型</option><option value="oauth" ${accountFilters.kind === "oauth" ? "selected" : ""}>OAuth</option><option value="setup_token" ${accountFilters.kind === "setup_token" ? "selected" : ""}>Setup Token</option><option value="api_key" ${accountFilters.kind === "api_key" ? "selected" : ""}>API Key</option></select>
      <select id="account-status-filter" aria-label="账号状态"><option value="">全部状态</option><option value="active" ${accountFilters.status === "active" ? "selected" : ""}>正常</option><option value="inactive" ${accountFilters.status === "inactive" ? "selected" : ""}>停用</option><option value="cooldown" ${accountFilters.status === "cooldown" ? "selected" : ""}>冷却中</option><option value="error" ${accountFilters.status === "error" ? "selected" : ""}>错误</option></select>
      <select id="account-group-filter" aria-label="路由分组"><option value="">全部分组</option><option value="ungrouped" ${accountFilters.group === "ungrouped" ? "selected" : ""}>未分组</option>${currentGroups.map(group => `<option value="${group.id}" ${String(accountFilters.group) === String(group.id) ? "selected" : ""}>${escapeHtml(group.name)}</option>`).join("")}</select>
    </div>
    <div class="account-toolbar-actions">
      <button class="button secondary icon-only" id="refresh-accounts" type="button" title="刷新账号" aria-label="刷新账号">${appIcon("refresh")}</button>
      <div class="account-auto-refresh-dropdown" id="account-auto-refresh-dropdown"><button class="button secondary account-auto-refresh-toggle" id="account-auto-refresh-toggle" type="button" aria-label="设置自动刷新" aria-expanded="false">${appIcon("refresh", accountAutoRefreshEnabled ? "account-refresh-spinning" : "")}<span class="account-refresh-label" id="account-refresh-countdown">${accountAutoRefreshEnabled ? `${accountAutoRefreshSeconds} 秒后刷新` : "自动刷新"}</span></button><div class="account-auto-refresh-menu" id="account-auto-refresh-menu" hidden>
        <button class="${accountAutoRefreshEnabled ? "active" : ""}" id="account-refresh-enabled" type="button"><span>启用自动刷新</span>${appIcon("check")}</button>
        <div class="account-menu-divider"></div>
        ${accountAutoRefreshOptions.map(seconds => `<button class="${accountAutoRefreshSeconds === seconds ? "active" : ""}" data-account-refresh-value="${seconds}" type="button"><span>每 ${seconds} 秒</span>${appIcon("check")}</button>`).join("")}
      </div></div>
      <div class="account-tools-dropdown" id="account-tools-dropdown"><button class="button secondary account-tools-toggle" id="account-tools-toggle" type="button" aria-label="更多操作" aria-expanded="false">${appIcon("more")}<span>更多操作</span>${appIcon("chevronDown", "account-tools-chevron")}</button><div class="account-tools-menu" id="account-tools-menu" hidden>
        <section><p>数据操作</p>
          <button id="sync-crs-accounts" type="button">${appIcon("refresh")}<span><strong>从 CRS 同步</strong><small>预览并同步 Claude 与 OpenAI 账号</small></span></button>
          <button id="import-accounts" type="button">${appIcon("upload")}<span><strong>导入账号</strong><small>导入 Sub2API JSON 备份</small></span></button>
          <button id="export-accounts" type="button">${appIcon("download")}<span><strong>${selectedAccountIds.size ? "导出选中" : "导出账号"}</strong><small>${selectedAccountIds.size ? `已选择 ${selectedAccountIds.size} 个账号` : "导出含凭证的敏感备份"}</small></span></button>
        </section>
        <section><p>工具</p>
          <button id="error-passthrough-rules" type="button">${appIcon("shield")}<span><strong>错误透传规则</strong><small>控制上游错误响应与监控</small></span></button>
          <button id="tls-fingerprint-profiles" type="button">${appIcon("lock")}<span><strong>TLS 指纹模板</strong><small>管理账号使用的 TLS 参数</small></span></button>
        </section>
        <section class="account-column-menu"><p><span>列显示</span>${appIcon("grid")}</p>${accountOptionalColumns.map(column => `<button class="${accountColumnVisible(column.key) ? "active" : ""}" data-account-column-toggle="${column.key}" type="button"><span>${escapeHtml(column.label)}</span>${appIcon("check")}</button>`).join("")}</section>
      </div></div>
      <button class="button" id="add-account" type="button">${appIcon("plus")}<span>添加账号</span></button>
    </div>
  </section>
  <div id="account-collection"></div>`;
page.querySelector("#add-account").addEventListener("click", () => openAccountModal());
page.querySelector("#sync-crs-accounts").addEventListener("click", () => window.Sub2MiniAccountTools.openCrsSync());
page.querySelector("#import-accounts").addEventListener("click", openAccountImportModal);
page.querySelector("#export-accounts").addEventListener("click", () => exportAccounts([...selectedAccountIds]));
page.querySelector("#error-passthrough-rules").addEventListener("click", () => window.Sub2MiniAccountTools.openErrorRules());
page.querySelector("#tls-fingerprint-profiles").addEventListener("click", () => window.Sub2MiniAccountTools.openTlsProfiles());
page.querySelector("#refresh-accounts").addEventListener("click", renderRoute);
page.querySelector("#account-tools-toggle").addEventListener("click", () => {
  closeAccountAutoRefreshMenu();
  const menu = document.querySelector("#account-tools-menu");
  const opening = menu.hidden;
  if (!opening) return closeAccountToolsMenu();
  menu.hidden = false;
  positionAccountToolsMenu(page.querySelector("#account-tools-toggle"), menu);
  page.querySelector("#account-tools-toggle").setAttribute("aria-expanded", "true");
});
page.querySelector("#account-auto-refresh-toggle").addEventListener("click", () => {
  closeAccountToolsMenu();
  const menu = page.querySelector("#account-auto-refresh-menu");
  menu.hidden = !menu.hidden;
  page.querySelector("#account-auto-refresh-toggle").setAttribute("aria-expanded", String(!menu.hidden));
});
page.querySelector("#account-search").addEventListener("input", event => updateAccountFilter(page, "search", event.currentTarget.value));
page.querySelector("#account-platform-filter").addEventListener("change", event => updateAccountFilter(page, "platform", event.currentTarget.value));
page.querySelector("#account-kind-filter").addEventListener("change", event => updateAccountFilter(page, "kind", event.currentTarget.value));
page.querySelector("#account-status-filter").addEventListener("change", event => updateAccountFilter(page, "status", event.currentTarget.value));
page.querySelector("#account-group-filter").addEventListener("change", event => updateAccountFilter(page, "group", event.currentTarget.value));
page.querySelector("#account-refresh-enabled").addEventListener("click", () => setAccountAutoRefreshEnabled(page, !accountAutoRefreshEnabled));
page.querySelectorAll("[data-account-refresh-value]").forEach(button => button.addEventListener("click", event => setAccountAutoRefresh(page, Number(event.currentTarget.dataset.accountRefreshValue))));
page.querySelectorAll("[data-account-column-toggle]").forEach(input => input.addEventListener("click", event => {
  const key = event.currentTarget.dataset.accountColumnToggle;
  hiddenAccountColumns.has(key) ? hiddenAccountColumns.delete(key) : hiddenAccountColumns.add(key);
  localStorage.setItem(accountColumnStorageKey, JSON.stringify([...hiddenAccountColumns]));
  localStorage.setItem(accountColumnVersionKey, accountColumnVersion);
  event.currentTarget.classList.toggle("active", accountColumnVisible(key));
  renderAccountCollection(page);
}));
page.querySelectorAll("#account-tools-menu button").forEach(button => button.addEventListener("click", closeAccountToolsMenu));
renderAccountCollection(page);
scheduleAccountAutoRefresh(page);
}
function renderAccountCollection(page) {
closeUpstreamAccountMenu();
updateAccountExportAction(page);
const accounts = sortedAccounts(filteredAccounts());
const pageCount = Math.max(1, Math.ceil(accounts.length / accountPageSize));
accountPage = Math.min(accountPage, pageCount);
const start = (accountPage - 1) * accountPageSize;
const visible = accounts.slice(start, start + accountPageSize);
const target = page.querySelector("#account-collection");
const hasFilters = Object.values(accountFilters).some(Boolean);
const subtitle = accounts.length === currentAccounts.length ? `${currentAccounts.length} 个账号 · Claude 与 OpenAI 上游连接及调度` : `显示 ${accounts.length} / ${currentAccounts.length} 个账号`;
page.querySelector(".page-header p").textContent = subtitle;
const topbarDescription = document.querySelector("#topbar-description");
if (topbarDescription && currentRouteName() === "accounts") { topbarDescription.textContent = subtitle; topbarDescription.hidden = false; }
target.innerHTML = `${currentAccounts.length ? accountBulkBar(accounts, visible) : ""}${accounts.length ? `${accountTable(visible)}${accountPagination(accounts.length, start, visible.length)}` : emptyState(currentAccounts.length ? "没有匹配账号" : "暂无账号", currentAccounts.length ? "调整筛选条件后重试" : "添加 Claude 或 OpenAI 账号后即可转发请求", currentAccounts.length ? (hasFilters ? "清除筛选" : "") : "添加账号", currentAccounts.length ? "reset-account-filters" : "empty-add-account")}<div id="account-menu-host">${visible.map(accountActionMenu).join("")}</div>`;
target.querySelector("#empty-add-account")?.addEventListener("click", () => openAccountModal());
target.querySelector("#reset-account-filters")?.addEventListener("click", () => {
  accountFilters = { search: "", platform: "", kind: "", status: "", group: "" };
  accountPage = 1;
  page.querySelector("#account-search").value = "";
  page.querySelector("#account-platform-filter").value = "";
  page.querySelector("#account-kind-filter").value = "";
  page.querySelector("#account-status-filter").value = "";
  page.querySelector("#account-group-filter").value = "";
  renderAccountCollection(page);
});
target.querySelectorAll("[data-account-action]").forEach(button => button.addEventListener("click", handleAccountAction));
target.querySelectorAll("[data-account-menu]").forEach(button => button.addEventListener("click", openUpstreamAccountMenu));
target.querySelectorAll("[data-account-batch]").forEach(button => button.addEventListener("click", applyAccountBatch));
window.Sub2MiniAccountTools.attach(target);
window.Sub2MiniAccountSchedules.attach(target);
target.querySelectorAll("[data-account-select]").forEach(input => input.addEventListener("change", event => {
  const id = Number(event.currentTarget.value);
  event.currentTarget.checked ? selectedAccountIds.add(id) : selectedAccountIds.delete(id);
  renderAccountCollection(page);
}));
target.querySelectorAll("[data-account-select-all]").forEach(input => input.addEventListener("change", event => {
  visible.forEach(account => event.currentTarget.checked ? selectedAccountIds.add(account.id) : selectedAccountIds.delete(account.id));
  renderAccountCollection(page);
}));
target.querySelector("#select-account-page")?.addEventListener("click", () => {
  visible.forEach(account => selectedAccountIds.add(account.id));
  renderAccountCollection(page);
});
target.querySelector("#clear-account-selection")?.addEventListener("click", () => {
  selectedAccountIds.clear();
  renderAccountCollection(page);
});
target.querySelectorAll("[data-account-sort]").forEach(button => button.addEventListener("click", event => {
  const key = event.currentTarget.dataset.accountSort;
  accountSort = { key, order: accountSort.key === key && accountSort.order === "asc" ? "desc" : "asc" };
  localStorage.setItem(accountSortStorageKey, JSON.stringify(accountSort));
  accountPage = 1;
  renderAccountCollection(page);
}));
target.querySelector("#account-page-prev")?.addEventListener("click", () => { accountPage -= 1; renderAccountCollection(page); });
target.querySelector("#account-page-next")?.addEventListener("click", () => { accountPage += 1; renderAccountCollection(page); });
target.querySelector("#account-page-size")?.addEventListener("change", event => {
  accountPageSize = Number(event.currentTarget.value);
  localStorage.setItem("mini_account_page_size", String(accountPageSize));
  accountPage = 1;
  renderAccountCollection(page);
});
}
function updateAccountExportAction(page) {
const button = page.querySelector("#export-accounts");
if (!button) return;
button.innerHTML = `${appIcon("download")}<span><strong>${selectedAccountIds.size ? "导出选中" : "导出账号"}</strong><small>${selectedAccountIds.size ? `已选择 ${selectedAccountIds.size} 个账号` : "导出含凭证的敏感备份"}</small></span>`;
}
function updateAccountFilter(page, key, value) {
accountFilters[key] = value;
accountPage = 1;
renderAccountCollection(page);
}
function filteredAccounts() {
const query = accountFilters.search.trim().toLowerCase();
return currentAccounts.filter(account => {
  const groups = accountGroups(account.id);
  const stateName = accountState(account);
  if (query && ![account.id, account.name, account.email, account.base_url, account.proxy_name, account.notes].some(value => String(value || "").toLowerCase().includes(query))) return false;
  if (accountFilters.platform && account.platform !== accountFilters.platform) return false;
  if (accountFilters.kind && (accountFilters.kind === "setup_token" ? account.account_type !== "setup_token" : account.kind !== accountFilters.kind)) return false;
  if (accountFilters.status && stateName !== accountFilters.status) return false;
  if (accountFilters.group === "ungrouped" && groups.length) return false;
  if (accountFilters.group && accountFilters.group !== "ungrouped" && !groups.some(group => String(group.id) === String(accountFilters.group))) return false;
  return true;
});
}
function sortedAccounts(accounts) {
const direction = accountSort.order === "desc" ? -1 : 1;
return [...accounts].sort((left, right) => {
  const a = accountSortValue(left, accountSort.key);
  const b = accountSortValue(right, accountSort.key);
  if (a == null && b == null) return left.id - right.id;
  if (a == null) return 1;
  if (b == null) return -1;
  const result = typeof a === "number" && typeof b === "number" ? a - b : String(a).localeCompare(String(b), "zh-CN", { numeric: true, sensitivity: "base" });
  return result === 0 ? left.id - right.id : result * direction;
});
}
function accountSortValue(account, key) {
if (key === "status") return accountState(account);
if (key === "schedulable") return account.enabled ? 1 : 0;
if (["id", "priority"].includes(key)) return Number(account[key]);
if (["last_used_at", "created_at", "expires_at"].includes(key)) return account[key] ? new Date(account[key]).getTime() : null;
return account[key] ?? "";
}
function accountGroups(accountId) {
return currentGroups.filter(group => (group.account_ids || []).some(id => Number(id) === Number(accountId)));
}
function accountState(account) {
if (!account.enabled) return "inactive";
if (account.cooldown_until) return "cooldown";
if (account.last_error) return "error";
return "active";
}
function accountStatus(account) {
const value = accountState(account);
if (value === "inactive") return status("停用", "off");
if (value === "cooldown") return `${status("冷却中", "warn")}<span class="cell-sub">至 ${formatDate(account.cooldown_until)}</span>`;
if (value === "error") return `${status("错误", "error")}<span class="cell-sub" title="${escapeHtml(account.last_error)}">${escapeHtml(account.last_error)}</span>`;
return status("正常");
}
function accountBulkBar(accounts, visible) {
const selected = selectedAccountIds.size;
return `<section class="account-bulk-bar" aria-label="批量账号操作"><div><strong>${selected ? `已选择 ${selected} 个账号` : "批量编辑"}</strong>${selected ? `<button class="account-selection-link" id="select-account-page" type="button">选择当前页</button><span class="account-selection-dot">•</span><button class="account-selection-link" id="clear-account-selection" type="button">清除</button>` : ""}</div><div class="account-bulk-actions">${selected ? `<button class="button danger small" data-account-batch="delete">删除</button><button class="button secondary small" data-account-batch="recover">重置状态</button><button class="button secondary small" data-account-batch="refresh">刷新令牌</button><button class="button success small" data-account-batch="enable">启用调度</button><button class="button warning small" data-account-batch="disable">停止调度</button><button class="button small" data-account-batch="edit">编辑</button>` : ""}<button class="button small" data-account-batch="edit-filtered" data-filtered-ids="${accounts.map(account => account.id).join(",")}" ${visible.length ? "" : "disabled"}>批量修改</button></div></section>`;
}
function accountTable(accounts) {
return `<div class="table-wrap account-table-wrap"><table class="account-table">
  <thead><tr><th><input type="checkbox" data-account-select-all aria-label="选择当前页账号" ${accounts.length && accounts.every(account => selectedAccountIds.has(account.id)) ? "checked" : ""}></th><th>${accountSortHeader("name", "账号")}</th>${accountTableColumn("id", `<th>${accountSortHeader("id", "ID")}</th>`)}${accountTableColumn("platform_type", "<th>平台 / 类型</th>")}${accountTableColumn("capacity", "<th>容量</th>")}${accountTableColumn("status", `<th>${accountSortHeader("status", "状态")}</th>`)}${accountTableColumn("schedulable", `<th>${accountSortHeader("schedulable", "可调度")}</th>`)}${accountTableColumn("today_stats", "<th>今日统计</th>")}${accountTableColumn("groups", "<th>分组</th>")}${accountTableColumn("usage", "<th>用量窗口</th>")}${accountTableColumn("proxy", "<th>代理</th>")}${accountTableColumn("priority", `<th>${accountSortHeader("priority", "优先级")}</th>`)}${accountTableColumn("last_used_at", `<th>${accountSortHeader("last_used_at", "最近使用")}</th>`)}${accountTableColumn("created_at", `<th>${accountSortHeader("created_at", "创建时间")}</th>`)}${accountTableColumn("expires_at", `<th>${accountSortHeader("expires_at", "过期时间")}</th>`)}${accountTableColumn("notes", "<th>备注</th>")}<th>操作</th></tr></thead>
  <tbody>${accounts.map(account => `<tr>
    <td><input type="checkbox" data-account-select value="${account.id}" aria-label="选择 ${escapeHtml(account.name)}" ${selectedAccountIds.has(account.id) ? "checked" : ""}></td>
    <td><span class="cell-main">${escapeHtml(account.name)}</span>${account.email ? `<span class="cell-sub">${escapeHtml(account.email)}</span>` : ""}${account.parent_account_id ? `<span class="cell-sub">继承账号 #${account.parent_account_id}</span>` : ""}</td>
    ${accountTableColumn("id", `<td><span class="mono cell-sub">#${account.id}</span></td>`)}
    ${accountTableColumn("platform_type", `<td>${accountType(account)}</td>`)}
    ${accountTableColumn("capacity", `<td><span class="cell-main">${account.current_concurrency || 0} / ${account.concurrency}</span><span class="cell-sub">当前 / 上限</span></td>`)}
    ${accountTableColumn("status", `<td>${accountStatus(account)}</td>`)}
    ${accountTableColumn("schedulable", `<td>${accountScheduleSwitch(account)}</td>`)}
    ${accountTableColumn("today_stats", `<td>${accountTodayStats(account)}</td>`)}
    ${accountTableColumn("groups", `<td>${accountGroupBadges(account.id)}</td>`)}
    ${accountTableColumn("usage", `<td>${accountUsageWindow(account)}</td>`)}
    ${accountTableColumn("proxy", `<td>${accountProxy(account)}</td>`)}
    ${accountTableColumn("priority", `<td><span class="cell-main">${account.priority}</span></td>`)}
    ${accountTableColumn("last_used_at", `<td><span class="cell-sub">${formatDate(account.last_used_at)}</span></td>`)}
    ${accountTableColumn("created_at", `<td><span class="cell-sub">${formatDate(account.created_at)}</span></td>`)}
    ${accountTableColumn("expires_at", `<td><span class="cell-sub">${formatDate(account.expires_at)}</span></td>`)}
    ${accountTableColumn("notes", `<td><span class="account-notes" title="${escapeHtml(account.notes || "")}">${escapeHtml(account.notes || "-")}</span></td>`)}
    <td><div class="account-primary-actions"><button class="account-icon-action" data-account-action="edit" data-id="${account.id}" type="button">${appIcon("edit")}<span>编辑</span></button><button class="account-icon-action danger" data-account-action="delete" data-id="${account.id}" type="button">${appIcon("trash")}<span>删除</span></button><button class="account-icon-action" data-account-menu="${account.id}" type="button" aria-haspopup="menu" aria-expanded="false">${appIcon("more")}<span>更多</span></button></div></td>
  </tr>`).join("")}</tbody></table></div>${accountMobileList(accounts)}`;
}
function accountSortHeader(key, label) {
const active = accountSort.key === key;
return `<button class="account-sort ${active ? "active" : ""}" data-account-sort="${key}" type="button"><span>${label}</span><span aria-hidden="true">${active ? (accountSort.order === "asc" ? "↑" : "↓") : "↕"}</span></button>`;
}
function accountTableColumn(key, content) {
return accountColumnVisible(key) ? content : "";
}
function accountColumnVisible(key) {
return !hiddenAccountColumns.has(key);
}
function storedAccountHiddenColumns() {
try {
  const raw = localStorage.getItem(accountColumnStorageKey);
  const defaults = ["today_stats", "proxy", "priority", "notes"];
  if (!raw) {
    localStorage.setItem(accountColumnVersionKey, accountColumnVersion);
    return new Set(defaults);
  }
  const stored = JSON.parse(raw);
  const valid = new Set(accountOptionalColumns.map(column => column.key));
  const hidden = new Set(Array.isArray(stored) ? stored.filter(key => valid.has(key)) : []);
  if (localStorage.getItem(accountColumnVersionKey) !== accountColumnVersion) {
    defaults.forEach(key => hidden.add(key));
    localStorage.setItem(accountColumnStorageKey, JSON.stringify([...hidden]));
    localStorage.setItem(accountColumnVersionKey, accountColumnVersion);
  }
  return hidden;
} catch (_) { return new Set(["today_stats", "proxy", "priority", "notes"]); }
}
function storedAccountSort() {
try {
  const value = JSON.parse(localStorage.getItem(accountSortStorageKey) || "{}");
  const keys = new Set(["id", "name", "status", "schedulable", "priority", "last_used_at", "created_at", "expires_at"]);
  return keys.has(value.key) ? { key: value.key, order: value.order === "desc" ? "desc" : "asc" } : { key: "name", order: "asc" };
} catch (_) { return { key: "name", order: "asc" }; }
}
function accountType(account) {
const platform = account.platform === "anthropic" ? "Anthropic" : "OpenAI";
const type = account.account_type === "setup_token" ? "Setup Token" : account.kind === "oauth" ? `OAuth${account.parent_account_id ? " · Spark" : ""}` : "API Key";
return `<div class="account-type-stack"><span class="account-platform-badge ${account.platform === "anthropic" ? "anthropic" : ""}">${platform}</span><span>${type}</span></div>`;
}
function accountTodayStats(account) {
const stats = account.today_stats || {};
return `<span class="cell-main">${formatNumber(stats.requests || 0)} 次</span><span class="cell-sub">${formatNumber(stats.tokens || 0)} Token</span><span class="cell-sub">${formatUsdMicros(stats.cost_microusd || 0)}</span>`;
}
function accountUsageWindow(account) {
if (account.kind === "oauth") return `<span class="cell-main">OAuth Token</span><span class="cell-sub">${account.expires_at ? `到期 ${formatDate(account.expires_at)}` : "未提供过期时间"}</span>`;
const stats = account.today_stats || {};
return `<span class="cell-main">今日 ${formatNumber(stats.tokens || 0)} Token</span><span class="cell-sub">${formatUsdMicros(stats.cost_microusd || 0)}</span>`;
}
function accountScheduleSwitch(account) {
return `<button class="account-schedule-switch ${account.enabled ? "on" : ""}" data-account-action="toggle" data-id="${account.id}" data-enabled="${account.enabled}" type="button" role="switch" aria-checked="${account.enabled}" title="${account.enabled ? "停止调度" : "启用调度"}"><span></span></button>`;
}
function accountProxy(account) {
return account.proxy_name ? `<span class="cell-main">${escapeHtml(account.proxy_name)}</span><span class="cell-sub">${account.proxy_active ? "可用" : "不可用"}</span>` : `<span class="cell-sub">直连</span>`;
}
function accountMobileList(accounts) {
return `<div class="account-mobile-list"><label class="account-mobile-select-all"><input type="checkbox" data-account-select-all><span>选择当前账号</span><small>${accounts.length} 个账号</small></label>${accounts.map(account => `<article class="account-mobile-card">
  <header><input type="checkbox" data-account-select value="${account.id}" aria-label="选择 ${escapeHtml(account.name)}" ${selectedAccountIds.has(account.id) ? "checked" : ""}><div class="account-mobile-identity"><strong>${escapeHtml(account.name)}</strong><span>${escapeHtml(account.email || account.base_url)}</span>${account.parent_account_id ? `<small>继承账号 #${account.parent_account_id}</small>` : ""}</div>${accountColumnVisible("status") ? `<div class="account-mobile-status">${accountStatus(account)}</div>` : ""}</header>
  <dl>${accountMobileRow("id", "ID", `<span class="mono">#${account.id}</span>`)}${accountMobileRow("platform_type", "平台 / 类型", accountType(account))}${accountMobileRow("capacity", "容量", `<span>${account.current_concurrency || 0} / ${account.concurrency}</span>`)}${accountMobileRow("schedulable", "可调度", accountScheduleSwitch(account))}${accountMobileRow("today_stats", "今日统计", accountTodayStats(account))}${accountMobileRow("groups", "分组", accountGroupBadges(account.id))}${accountMobileRow("usage", "用量窗口", accountUsageWindow(account))}${accountMobileRow("proxy", "代理", accountProxy(account))}${accountMobileRow("priority", "优先级", `<span>${account.priority}</span>`)}${accountMobileRow("last_used_at", "最近使用", `<span>${formatDate(account.last_used_at)}</span>`)}${accountMobileRow("created_at", "创建时间", `<span>${formatDate(account.created_at)}</span>`)}${accountMobileRow("expires_at", "过期时间", `<span>${formatDate(account.expires_at)}</span>`)}${accountMobileRow("notes", "备注", `<span>${escapeHtml(account.notes || "-")}</span>`)}</dl>
  <footer><button class="account-icon-action" data-account-action="edit" data-id="${account.id}" type="button">${appIcon("edit")}<span>编辑</span></button><button class="account-icon-action danger" data-account-action="delete" data-id="${account.id}" type="button">${appIcon("trash")}<span>删除</span></button><button class="account-icon-action" data-account-menu="${account.id}" type="button" aria-haspopup="menu" aria-expanded="false">${appIcon("more")}<span>更多</span></button></footer>
</article>`).join("")}</div>`;
}
function accountMobileRow(key, label, value) {
return accountColumnVisible(key) ? `<div><dt>${label}</dt><dd>${value}</dd></div>` : "";
}
function accountGroupBadges(accountId) {
const groups = accountGroups(accountId);
if (!groups.length) return `<span class="cell-sub">未分组</span>`;
return `<div class="account-group-list">${groups.slice(0, 3).map(group => `<span title="${escapeHtml(group.name)}">${escapeHtml(group.name)}</span>`).join("")}${groups.length > 3 ? `<span>+${groups.length - 3}</span>` : ""}</div>`;
}
function accountPagination(total, start, visibleCount) {
const pages = Math.max(1, Math.ceil(total / accountPageSize));
return `<nav class="account-pagination" aria-label="账号分页"><span>显示 ${start + 1}-${start + visibleCount}，共 ${total} 条</span><div><label>每页 <select id="account-page-size">${[10, 20, 50, 100].map(size => `<option value="${size}" ${accountPageSize === size ? "selected" : ""}>${size}</option>`).join("")}</select></label><button class="button secondary small" id="account-page-prev" ${accountPage <= 1 ? "disabled" : ""} type="button">上一页</button><strong>${accountPage} / ${pages}</strong><button class="button secondary small" id="account-page-next" ${accountPage >= pages ? "disabled" : ""} type="button">下一页</button></div></nav>`;
}
function accountActionMenu(account) {
const inherited = Boolean(account.parent_account_id);
const hasShadow = currentAccounts.some(item => Number(item.parent_account_id) === Number(account.id));
return `<div class="account-action-popover" data-account-action-menu="${account.id}" role="menu" hidden>
  <button data-account-action="test" data-id="${account.id}" type="button">${appIcon("play")}<span>测试连接</span></button>
  <button data-account-tool="stats" data-id="${account.id}" type="button">${appIcon("chart")}<span>查看统计</span></button>
  <button data-account-schedules="${account.id}" type="button">${appIcon("clock")}<span>定时测试</span></button>
  ${account.kind === "api_key" && !inherited ? `<button data-account-tool="duplicate" data-id="${account.id}" type="button">${appIcon("copy")}<span>复制账号</span></button>` : ""}
  ${account.kind === "oauth" && !inherited ? `${account.platform === "openai" ? `<button data-account-tool="reauth" data-id="${account.id}" type="button">${appIcon("link")}<span>重新授权</span></button>` : ""}<button data-account-action="refresh" data-id="${account.id}" type="button">${appIcon("refresh")}<span>刷新令牌</span></button>` : ""}
  ${account.platform === "openai" && account.kind === "oauth" && !inherited && !hasShadow ? `<button data-account-tool="spark" data-id="${account.id}" type="button">${appIcon("copy")}<span>创建 Spark 影子</span></button>` : ""}
  ${account.cooldown_until || account.last_error ? `<div class="account-menu-divider"></div><button data-account-action="recover" data-id="${account.id}" type="button">${appIcon("refresh")}<span>恢复状态</span></button>` : ""}
</div>`;
}
function openUpstreamAccountMenu(event) {
const button = event.currentTarget;
const menu = document.querySelector(`[data-account-action-menu="${button.dataset.accountMenu}"]`);
if (!menu) return;
if (activeUpstreamAccountMenu === menu) { closeUpstreamAccountMenu(); return; }
closeUpstreamAccountMenu();
activeUpstreamAccountMenu = menu;
activeUpstreamAccountMenuTrigger = button;
document.body.append(menu);
menu.hidden = false;
button.setAttribute("aria-expanded", "true");
const rect = button.getBoundingClientRect();
const width = menu.offsetWidth;
const height = menu.offsetHeight;
const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
const top = rect.bottom + height + 8 <= window.innerHeight ? rect.bottom + 6 : Math.max(8, rect.top - height - 6);
menu.style.left = `${left}px`;
menu.style.top = `${top}px`;
}
function closeUpstreamAccountMenu() {
if (!activeUpstreamAccountMenu) return;
activeUpstreamAccountMenu.hidden = true;
activeUpstreamAccountMenu.removeAttribute("style");
activeUpstreamAccountMenuTrigger?.setAttribute("aria-expanded", "false");
const host = document.querySelector("#account-menu-host");
if (host) host.append(activeUpstreamAccountMenu); else activeUpstreamAccountMenu.remove();
activeUpstreamAccountMenu = null;
activeUpstreamAccountMenuTrigger = null;
}
function closeAccountToolsMenu() {
const menu = document.querySelector("#account-tools-menu");
const toggle = document.querySelector("#account-tools-toggle");
if (menu) {
  menu.hidden = true;
  menu.removeAttribute("style");
  const host = document.querySelector("#account-tools-dropdown");
  if (host && !host.contains(menu)) host.append(menu);
}
toggle?.setAttribute("aria-expanded", "false");
}
function positionAccountToolsMenu(trigger, menu) {
const rect = trigger.getBoundingClientRect();
const width = Math.min(320, window.innerWidth - 16);
const availableBelow = window.innerHeight - rect.bottom - 14;
const openBelow = availableBelow >= 220;
const maxHeight = Math.min(560, openBelow ? availableBelow : rect.top - 14);
const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
document.body.append(menu);
menu.style.position = "fixed";
menu.style.width = `${width}px`;
menu.style.maxHeight = `${Math.max(180, maxHeight)}px`;
menu.style.left = `${left}px`;
menu.style.right = "auto";
menu.style.top = openBelow ? `${rect.bottom + 6}px` : "auto";
menu.style.bottom = openBelow ? "auto" : `${window.innerHeight - rect.top + 6}px`;
}
function closeAccountAutoRefreshMenu() {
const menu = document.querySelector("#account-auto-refresh-menu");
const toggle = document.querySelector("#account-auto-refresh-toggle");
if (menu) menu.hidden = true;
toggle?.setAttribute("aria-expanded", "false");
}
function readStoredAccountAutoRefresh() {
try {
  const raw = localStorage.getItem("mini_account_auto_refresh");
  if (!raw) return { enabled: false, interval_seconds: 30 };
  if (/^\d+$/.test(raw)) {
    const interval = Number(raw);
    return { enabled: accountAutoRefreshOptions.includes(interval), interval_seconds: accountAutoRefreshOptions.includes(interval) ? interval : 30 };
  }
  const parsed = JSON.parse(raw);
  const interval = accountAutoRefreshOptions.includes(Number(parsed.interval_seconds)) ? Number(parsed.interval_seconds) : 30;
  return { enabled: parsed.enabled === true, interval_seconds: interval };
} catch (_) { return { enabled: false, interval_seconds: 30 }; }
}
function saveAccountAutoRefresh() {
localStorage.setItem("mini_account_auto_refresh", JSON.stringify({ enabled: accountAutoRefreshEnabled, interval_seconds: accountAutoRefreshSeconds }));
}
function setAccountAutoRefreshEnabled(page, enabled) {
accountAutoRefreshEnabled = Boolean(enabled);
saveAccountAutoRefresh();
closeAccountAutoRefreshMenu();
renderAccounts(page).catch(error => toast(error.message, true));
}
function setAccountAutoRefresh(page, seconds) {
accountAutoRefreshSeconds = accountAutoRefreshOptions.includes(seconds) ? seconds : 30;
saveAccountAutoRefresh();
page.querySelectorAll("[data-account-refresh-value]").forEach(button => button.classList.toggle("active", Number(button.dataset.accountRefreshValue) === accountAutoRefreshSeconds));
closeAccountAutoRefreshMenu();
scheduleAccountAutoRefresh(page);
}
function scheduleAccountAutoRefresh(page) {
stopAccountAutoRefresh();
const countdown = page.querySelector("#account-refresh-countdown");
if (!accountAutoRefreshEnabled) {
  if (countdown) countdown.textContent = "自动刷新";
  return;
}
accountAutoRefreshDeadline = Date.now() + accountAutoRefreshSeconds * 1000;
const tick = () => {
  const remaining = Math.max(0, Math.ceil((accountAutoRefreshDeadline - Date.now()) / 1000));
  if (countdown?.isConnected) countdown.textContent = `${remaining} 秒后刷新`;
  if (remaining === 0) {
    if (modal.open || !document.querySelector("#account-tools-menu")?.hidden || !document.querySelector("#account-auto-refresh-menu")?.hidden || activeUpstreamAccountMenu) {
      accountAutoRefreshDeadline = Date.now() + accountAutoRefreshSeconds * 1000;
      return;
    }
    stopAccountAutoRefresh();
    if (currentRouteName() === "accounts") renderRoute();
  }
};
tick();
accountAutoRefreshTimer = window.setInterval(tick, 1000);
}
function stopAccountAutoRefresh() {
if (accountAutoRefreshTimer != null) window.clearInterval(accountAutoRefreshTimer);
accountAutoRefreshTimer = null;
accountAutoRefreshDeadline = 0;
}
async function applyAccountBatch(event) {
const action = event.currentTarget.dataset.accountBatch;
const ids = action === "edit-filtered"
  ? String(event.currentTarget.dataset.filteredIds || "").split(",").filter(Boolean).map(Number)
  : [...selectedAccountIds];
if (!ids.length) return;
if (action === "edit" || action === "edit-filtered") return openAccountBulkEditModal(ids);
if (["refresh", "delete"].includes(action) && !confirm(action === "delete" ? `确认删除所选 ${ids.length} 个账号？使用日志会保留。` : `确认刷新所选账号中的 OAuth Token？`)) return;
event.currentTarget.disabled = true;
try {
  const request = action === "enable" || action === "disable"
    ? ["/api/admin/accounts/bulk-update", { account_ids: ids, enabled: action === "enable" }]
    : action === "recover"
      ? ["/api/admin/accounts/batch-clear-error", { account_ids: ids }]
      : action === "refresh"
        ? ["/api/admin/accounts/batch-refresh", { account_ids: ids }]
        : ["/api/admin/accounts/batch-delete", { account_ids: ids }];
  const result = await api(request[0], { method: "POST", body: JSON.stringify(request[1]) });
  toast(result.data.failed ? `完成 ${result.data.success} 个，失败 ${result.data.failed} 个` : `已处理 ${result.data.success} 个账号`, result.data.failed > 0);
  await renderRoute();
} catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
}
function openAccountBulkEditModal(ids) {
openModal(`批量编辑 ${ids.length} 个账号`, `<form id="account-bulk-edit-form">
  <div class="form-grid"><div class="field"><label for="bulk-account-enabled">调度状态</label><select id="bulk-account-enabled"><option value="">保持不变</option><option value="true">启用</option><option value="false">停用</option></select></div><div class="field"><label for="bulk-account-proxy">网络代理</label><select id="bulk-account-proxy"><option value="__keep__">保持不变</option>${proxyOptions()}</select></div></div>
  <div class="form-grid"><div class="field"><label for="bulk-account-priority">优先级</label><input id="bulk-account-priority" type="number" min="0" placeholder="保持不变"></div><div class="field"><label for="bulk-account-concurrency">并发上限</label><input id="bulk-account-concurrency" type="number" min="1" max="1000" placeholder="保持不变"></div></div>
  <label class="toggle-line"><input id="bulk-account-replace-groups" type="checkbox"> 替换账号的路由分组</label>
  <div id="bulk-account-groups" class="choice-grid" hidden>${currentGroups.map(group => `<label><input type="checkbox" name="bulk_group_id" value="${group.id}"><span>${escapeHtml(group.name)}</span><small>${escapeHtml(group.platform_label || group.platform || "OpenAI")}</small></label>`).join("") || `<span class="field-hint">暂无路由分组；启用替换后将清空现有绑定</span>`}</div>
  <p class="form-error" id="account-bulk-edit-error"></p>
</form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-account-bulk-edit">保存</button>`);
modal.querySelector("#bulk-account-replace-groups").addEventListener("change", event => { modal.querySelector("#bulk-account-groups").hidden = !event.currentTarget.checked; });
modal.querySelector("#save-account-bulk-edit").addEventListener("click", event => saveAccountBulkEdit(ids, event.currentTarget));
}
async function saveAccountBulkEdit(ids, button) {
const enabled = modal.querySelector("#bulk-account-enabled").value;
const proxy = modal.querySelector("#bulk-account-proxy").value;
const priority = modal.querySelector("#bulk-account-priority").value;
const concurrency = modal.querySelector("#bulk-account-concurrency").value;
const replaceGroups = modal.querySelector("#bulk-account-replace-groups").checked;
const payload = { account_ids: ids };
if (enabled) payload.enabled = enabled === "true";
if (proxy !== "__keep__") payload.proxy_id = proxy ? Number(proxy) : null;
if (priority !== "") payload.priority = Number(priority);
if (concurrency !== "") payload.concurrency = Number(concurrency);
if (replaceGroups) payload.group_ids = [...modal.querySelectorAll("[name=bulk_group_id]:checked")].map(input => Number(input.value));
if (Object.keys(payload).length === 1) { modal.querySelector("#account-bulk-edit-error").textContent = "至少选择一个要修改的字段"; return; }
button.disabled = true;
try {
  const result = await api("/api/admin/accounts/bulk-update", { method: "POST", body: JSON.stringify(payload) });
  closeModal(); toast(result.data.failed ? `更新 ${result.data.success} 个，失败 ${result.data.failed} 个` : `已更新 ${result.data.success} 个账号`, result.data.failed > 0); await renderRoute();
} catch (error) { modal.querySelector("#account-bulk-edit-error").textContent = error.message; button.disabled = false; }
}
let accountCreateState = null;
function openAccountModal() {
accountCreateState = {
  step: 1, platform: "anthropic", category: "oauth", addMethod: "oauth",
  name: "", notes: "", base_url: "https://api.anthropic.com", api_key: "",
  priority: 50, concurrency: 3, proxy_id: "", tls_fingerprint_profile_id: "",
  auth_json: "", auth_code: "", claude_session_id: ""
};
openModal("添加账号", "", "");
modal.classList.add("account-create-modal");
renderAccountCreateModal();
}
function accountCreateSteps(platform, step) {
const title = platform === "anthropic" ? "Claude 账号授权" : "OpenAI 账号授权";
return `<div class="account-create-steps" aria-label="添加账号步骤"><div class="active"><span>1</span><strong>授权方式</strong></div><i></i><div class="${step === 2 ? "active" : ""}"><span>2</span><strong>${title}</strong></div></div>`;
}
function accountCreateTypeCards(state) {
if (state.platform === "anthropic") return `<div class="account-create-types">
  <button class="account-create-type ${state.category === "oauth" ? "selected anthropic" : ""}" data-account-create-type="oauth" type="button"><span class="account-create-type-icon">${appIcon("sparkles")}</span><span><strong>Claude Code</strong><small>OAuth / Setup Token</small></span></button>
  <button class="account-create-type ${state.category === "api_key" ? "selected" : ""}" data-account-create-type="api_key" type="button"><span class="account-create-type-icon">${appIcon("key")}</span><span><strong>Claude Console</strong><small>API Key</small></span></button>
</div>`;
return `<div class="account-create-types two-column">
  <button class="account-create-type ${state.category === "oauth" ? "selected openai" : ""}" data-account-create-type="oauth" type="button"><span class="account-create-type-icon">${appIcon("link")}</span><span><strong>OAuth</strong><small>ChatGPT / Codex 授权</small></span></button>
  <button class="account-create-type ${state.category === "api_key" ? "selected" : ""}" data-account-create-type="api_key" type="button"><span class="account-create-type-icon">${appIcon("key")}</span><span><strong>API Key</strong><small>OpenAI API</small></span></button>
</div>`;
}
function accountCreateMethod(state) {
if (state.platform !== "anthropic" || state.category !== "oauth") return "";
return `<section class="account-create-section"><label>添加方式</label><div class="account-create-methods">
  <button class="${state.addMethod === "oauth" ? "selected" : ""}" data-account-create-method="oauth" type="button"><span>${appIcon("link")}</span><strong>OAuth</strong><small>完整 Claude Code 授权</small></button>
  <button class="${state.addMethod === "setup_token" ? "selected" : ""}" data-account-create-method="setup_token" type="button"><span>${appIcon("key")}</span><strong>Setup Token</strong><small>仅推理权限</small></button>
</div></section>`;
}
function accountCreateBasicBody(state) {
const oauth = state.category === "oauth";
const defaultUrl = state.platform === "anthropic" ? "https://api.anthropic.com" : "https://api.openai.com";
return `${oauth ? accountCreateSteps(state.platform, 1) : ""}<form id="account-create-form" class="account-create-form">
  <div class="field"><label for="account-name">账号名称</label><input id="account-name" data-account-create-field="name" value="${escapeHtml(state.name)}" placeholder="请输入账号名称" maxlength="120" required></div>
  <div class="field"><label for="account-notes">备注</label><textarea id="account-notes" data-account-create-field="notes" rows="3" maxlength="1000" placeholder="请输入备注">${escapeHtml(state.notes)}</textarea><span class="field-hint">备注可选</span></div>
  <section class="account-create-section"><label>平台</label><div class="account-create-platforms">
    <button class="${state.platform === "anthropic" ? "selected anthropic" : ""}" data-account-create-platform="anthropic" type="button">${appIcon("sparkles")}<span>Anthropic</span></button>
    <button class="${state.platform === "openai" ? "selected openai" : ""}" data-account-create-platform="openai" type="button">${appIcon("bolt")}<span>OpenAI</span></button>
  </div></section>
  <section class="account-create-section"><label>账号类型</label>${accountCreateTypeCards(state)}</section>
  ${accountCreateMethod(state)}
  ${oauth ? "" : `<section class="account-create-credentials"><div class="field"><label for="base-url">Base URL</label><input id="base-url" data-account-create-field="base_url" value="${escapeHtml(state.base_url || defaultUrl)}" type="url" required></div><div class="field"><label for="upstream-key">API Key</label><input id="upstream-key" data-account-create-field="api_key" value="${escapeHtml(state.api_key)}" type="password" autocomplete="new-password" placeholder="${state.platform === "anthropic" ? "sk-ant-..." : "sk-..."}" required></div></section>`}
  <details class="account-create-advanced"><summary>调度与网络设置</summary><div class="form-grid"><div class="field"><label for="priority">优先级</label><input id="priority" data-account-create-field="priority" type="number" min="0" value="${state.priority}" required></div><div class="field"><label for="concurrency">并发上限</label><input id="concurrency" data-account-create-field="concurrency" type="number" min="1" max="1000" value="${state.concurrency}" required></div></div><div class="form-grid"><div class="field"><label for="account-proxy">网络代理</label><select id="account-proxy" data-account-create-field="proxy_id">${proxyOptions(state.proxy_id)}</select></div><div class="field"><label for="account-tls-profile">TLS 指纹模板</label><select id="account-tls-profile" data-account-create-field="tls_fingerprint_profile_id">${tlsProfileOptions(state.tls_fingerprint_profile_id)}</select></div></div></details>
  <p class="form-error" id="account-error"></p>
</form>`;
}
function accountCreateAuthorizationBody(state) {
if (state.platform === "anthropic") return `${accountCreateSteps(state.platform, 2)}<div class="account-auth-panel"><div class="account-auth-mark anthropic">${appIcon("sparkles")}</div><h3>${state.addMethod === "setup_token" ? "获取 Claude Setup Token" : "授权 Claude Code 账号"}</h3><p>打开 Anthropic 授权页面完成登录，然后将页面返回的授权码粘贴到下方。</p><button class="button" id="start-claude-oauth" type="button">${appIcon("externalLink")}<span>${state.claude_session_id ? "重新打开授权页面" : "打开授权页面"}</span></button>${state.claude_session_id ? `<span class="account-auth-ready">授权会话已创建，有效期 30 分钟</span>` : ""}<div class="field"><label for="claude-auth-code">授权码</label><textarea id="claude-auth-code" data-account-create-field="auth_code" spellcheck="false" placeholder="code#state">${escapeHtml(state.auth_code)}</textarea></div><p class="form-error" id="account-error"></p></div>`;
return `${accountCreateSteps(state.platform, 2)}<div class="account-auth-panel"><div class="account-auth-mark openai">${appIcon("bolt")}</div><h3>OpenAI 账号授权</h3><p>可使用浏览器完成 Codex OAuth，或导入官方 Codex 的 auth.json。</p><button class="button" id="start-openai-oauth" type="button">${appIcon("externalLink")}<span>浏览器 OAuth</span></button><div class="account-auth-divider"><span>或者导入</span></div><div class="field"><label for="openai-auth-json">Codex auth.json</label><textarea id="openai-auth-json" data-account-create-field="auth_json" spellcheck="false" placeholder='{"tokens":{"access_token":"..."}}'>${escapeHtml(state.auth_json)}</textarea></div><p class="form-error" id="account-error"></p></div>`;
}
function bindAccountCreateFields() {
modal.querySelectorAll("[data-account-create-field]").forEach(input => input.addEventListener("input", event => {
  const key = event.currentTarget.dataset.accountCreateField;
  accountCreateState[key] = ["priority", "concurrency"].includes(key) ? Number(event.currentTarget.value) : event.currentTarget.value;
}));
modal.querySelectorAll("select[data-account-create-field]").forEach(input => input.addEventListener("change", event => { accountCreateState[event.currentTarget.dataset.accountCreateField] = event.currentTarget.value; }));
}
function renderAccountCreateModal() {
const state = accountCreateState;
modal.querySelector(".modal-body").innerHTML = state.step === 1 ? accountCreateBasicBody(state) : accountCreateAuthorizationBody(state);
modal.querySelector(".modal-footer")?.remove();
const footer = document.createElement("div"); footer.className = "modal-footer";
footer.innerHTML = state.step === 1
  ? `<button class="button secondary" data-close-modal type="button">取消</button><button class="button" id="account-create-next" type="button">${state.category === "oauth" ? "下一步" : "保存"}</button>`
  : `<button class="button secondary" id="account-create-back" type="button">上一步</button><button class="button" id="account-create-finish" type="button">${state.platform === "anthropic" ? "完成授权" : "导入 auth.json"}</button>`;
modal.append(footer);
bindAccountCreateFields();
modal.querySelectorAll("[data-close-modal]").forEach(button => button.addEventListener("click", closeModal));
modal.querySelectorAll("[data-account-create-platform]").forEach(button => button.addEventListener("click", event => {
  const platform = event.currentTarget.dataset.accountCreatePlatform;
  state.platform = platform; state.category = "oauth"; state.addMethod = "oauth";
  state.base_url = platform === "anthropic" ? "https://api.anthropic.com" : "https://api.openai.com";
  renderAccountCreateModal();
}));
modal.querySelectorAll("[data-account-create-type]").forEach(button => button.addEventListener("click", event => { state.category = event.currentTarget.dataset.accountCreateType; renderAccountCreateModal(); }));
modal.querySelectorAll("[data-account-create-method]").forEach(button => button.addEventListener("click", event => { state.addMethod = event.currentTarget.dataset.accountCreateMethod; renderAccountCreateModal(); }));
modal.querySelector("#account-create-next")?.addEventListener("click", saveAccountCreateStep);
modal.querySelector("#account-create-back")?.addEventListener("click", () => { state.step = 1; renderAccountCreateModal(); });
modal.querySelector("#account-create-finish")?.addEventListener("click", finishAccountAuthorization);
modal.querySelector("#start-claude-oauth")?.addEventListener("click", startClaudeOAuth);
modal.querySelector("#start-openai-oauth")?.addEventListener("click", startOpenAIOAuth);
}
function accountCreatePayload() {
const state = accountCreateState;
return { name: state.name.trim(), notes: state.notes.trim(), priority: Number(state.priority), concurrency: Number(state.concurrency), proxy_id: state.proxy_id ? Number(state.proxy_id) : null, tls_fingerprint_profile_id: state.tls_fingerprint_profile_id ? Number(state.tls_fingerprint_profile_id) : null };
}
async function saveAccountCreateStep() {
const form = modal.querySelector("#account-create-form");
if (!form.reportValidity()) return;
const state = accountCreateState;
if (state.category === "oauth") { state.step = 2; renderAccountCreateModal(); return; }
const button = modal.querySelector("#account-create-next"); button.disabled = true;
try {
  await api("/api/admin/accounts", { method: "POST", body: JSON.stringify({ ...accountCreatePayload(), kind: "api_key", platform: state.platform, account_type: "api_key", base_url: state.base_url, api_key: state.api_key }) });
  closeModal(); toast("账号已添加"); await renderRoute();
} catch (error) { modal.querySelector("#account-error").textContent = error.message; button.disabled = false; }
}
async function startClaudeOAuth() {
const popup = window.open("about:blank", "_blank");
try {
  const result = await api("/api/admin/claude/oauth/start", { method: "POST", body: JSON.stringify({ setup_token: accountCreateState.addMethod === "setup_token" }) });
  accountCreateState.claude_session_id = result.data.session_id;
  if (popup) { popup.opener = null; popup.location = result.data.auth_url; } else window.open(result.data.auth_url, "_blank", "noopener");
  renderAccountCreateModal(); toast("已打开 Claude 授权页面");
} catch (error) { popup?.close(); modal.querySelector("#account-error").textContent = error.message; }
}
async function startOpenAIOAuth() {
const button = modal.querySelector("#start-openai-oauth"); button.disabled = true;
try {
  const result = await api("/api/admin/oauth/start", { method: "POST", body: JSON.stringify(accountCreatePayload()) });
  window.open(result.data.auth_url, "_blank", "noopener"); closeModal(); toast("已打开 OpenAI OAuth 授权窗口");
} catch (error) { modal.querySelector("#account-error").textContent = error.message; button.disabled = false; }
}
async function finishAccountAuthorization() {
const state = accountCreateState; const button = modal.querySelector("#account-create-finish"); const error = modal.querySelector("#account-error");
if (state.platform === "anthropic" && (!state.claude_session_id || !state.auth_code.trim())) { error.textContent = state.claude_session_id ? "请输入授权码" : "请先打开授权页面"; return; }
if (state.platform === "openai" && !state.auth_json.trim()) { error.textContent = "请输入 auth.json 内容，或使用浏览器 OAuth"; return; }
button.disabled = true;
try {
  const payload = accountCreatePayload();
  if (state.platform === "anthropic") await api("/api/admin/claude/oauth/exchange", { method: "POST", body: JSON.stringify({ ...payload, session_id: state.claude_session_id, code: state.auth_code }) });
  else await api("/api/admin/oauth/import", { method: "POST", body: JSON.stringify({ ...payload, content: state.auth_json }) });
  closeModal(); toast("账号已添加"); await renderRoute();
} catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
}
async function handleAccountAction(event) {
const button = event.currentTarget;
const id = button.dataset.id;
const action = button.dataset.accountAction;
const account = currentAccounts.find(item => String(item.id) === String(id));
closeUpstreamAccountMenu();
if (action === "edit") {
  if (account) openAccountEditModal(account);
  return;
}
if (action === "delete" && !confirm(`确认删除账号“${account?.name || id}”？使用日志会保留。`)) return;
button.disabled = true;
try {
  if (action === "test") {
    const result = await api(`/api/admin/accounts/${id}/test`, { method: "POST", body: "{}" });
    toast(`连接正常，可见 ${result.data.models} 个模型`);
    return;
  }
  if (action === "refresh") await api(`/api/admin/accounts/${id}/refresh`, { method: "POST", body: "{}" });
  if (action === "recover") await api(`/api/admin/accounts/${id}/recover`, { method: "POST", body: "{}" });
  if (action === "toggle") await api(`/api/admin/accounts/${id}`, { method: "PUT", body: JSON.stringify({ enabled: button.dataset.enabled !== "true" }) });
  if (action === "delete") await api(`/api/admin/accounts/${id}`, { method: "DELETE" });
  toast(action === "delete" ? "账号已删除" : "账号已更新");
  await renderRoute();
} catch (error) { toast(error.message, true); }
finally { button.disabled = false; }
}
function openAccountEditModal(account) {
const inherited = Boolean(account.parent_account_id);
openModal("编辑上游账号", `<form id="account-edit-form">
  <div class="field"><label for="edit-account-name">名称</label><input id="edit-account-name" name="name" value="${escapeHtml(account.name)}" required autofocus></div>
  ${inherited ? `<div class="sensitive-notice"><strong>Spark 影子账号</strong><span>凭证、Base URL 和代理继承自母账号 #${account.parent_account_id}，此处只调整名称与调度参数。</span></div>` : `<div class="field"><label for="edit-base-url">Base URL</label><input id="edit-base-url" name="base_url" value="${escapeHtml(account.base_url)}" required></div>${account.kind === "api_key" ? `<div class="field"><label for="edit-upstream-key">替换上游 API Key</label><input id="edit-upstream-key" name="api_key" type="password" autocomplete="off"><span class="field-hint">留空保留当前密钥</span></div>` : ""}`}
  <div class="form-grid"><div class="field"><label for="edit-priority">优先级</label><input id="edit-priority" name="priority" type="number" min="0" value="${account.priority}" required></div><div class="field"><label for="edit-concurrency">并发上限</label><input id="edit-concurrency" name="concurrency" type="number" min="1" max="1000" value="${account.concurrency}" required></div></div>
  ${inherited ? "" : `<div class="field"><label for="edit-account-proxy">网络代理</label><select id="edit-account-proxy" name="proxy_id">${proxyOptions(account.proxy_id)}</select><span class="field-hint">选择直连可解除代理绑定</span></div>`}
  ${inherited ? "" : `<div class="field"><label for="edit-account-tls-profile">TLS 指纹模板</label><select id="edit-account-tls-profile" name="tls_fingerprint_profile_id">${tlsProfileOptions(account.tls_fingerprint_profile_id)}</select></div>`}
  <div class="field"><label for="edit-account-notes">备注</label><textarea class="compact-textarea" id="edit-account-notes" name="notes" maxlength="1000">${escapeHtml(account.notes || "")}</textarea></div>
  <p class="form-error" id="account-edit-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-account-edit">保存</button>`);
modal.querySelector("#save-account-edit").addEventListener("click", () => saveAccountEdit(account.id));
}
async function saveAccountEdit(id) {
const form = modal.querySelector("#account-edit-form");
if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
values.priority = Number(values.priority);
values.concurrency = Number(values.concurrency);
if ("proxy_id" in values) values.proxy_id = values.proxy_id ? Number(values.proxy_id) : null;
if ("tls_fingerprint_profile_id" in values) values.tls_fingerprint_profile_id = values.tls_fingerprint_profile_id ? Number(values.tls_fingerprint_profile_id) : null;
if (!values.api_key) delete values.api_key;
const button = modal.querySelector("#save-account-edit");
button.disabled = true;
try {
  await api(`/api/admin/accounts/${id}`, { method: "PUT", body: JSON.stringify(values) });
  closeModal(); toast("账号设置已更新"); await renderRoute();
} catch (error) { modal.querySelector("#account-edit-error").textContent = error.message; }
finally { button.disabled = false; }
}
function proxyOptions(selectedId = null) {
return `<option value="">直连</option>${currentProxies.map(proxy => `<option value="${proxy.id}" ${String(proxy.id) === String(selectedId) ? "selected" : ""}>${escapeHtml(proxy.name)} · ${escapeHtml(proxy.address)}${proxy.status === "active" ? "" : ` · ${escapeHtml(proxy.status)}`}</option>`).join("")}`;
}
function tlsProfileOptions(selectedId = null) {
return `<option value="">默认 TLS</option>${currentTlsProfiles.map(profile => `<option value="${profile.id}" ${String(profile.id) === String(selectedId) ? "selected" : ""}>${escapeHtml(profile.name)}</option>`).join("")}`;
}
function openAccountImportModal() {
openModal("导入账号备份", `<form id="account-import-form">
  <div class="sensitive-notice"><strong>敏感数据</strong><span>备份文件包含上游密钥、OAuth Token 和代理密码。仅导入可信文件，完成后妥善保管或删除文件。</span></div>
  <div class="field"><label for="account-import-files">JSON 备份文件</label><input id="account-import-files" name="files" type="file" accept="application/json,.json" multiple required><span class="field-hint">支持原版 sub2api-data / sub2api-bundle v1，可一次选择多个文件；Mini 仅导入 OpenAI API Key 与 Codex OAuth 账号。</span></div>
  <div id="account-import-result" class="import-result" hidden></div>
  <p class="form-error" id="account-import-error"></p>
</form>`, `<button class="button secondary" data-close-modal>关闭</button><button class="button" id="save-account-import">导入</button>`);
modal.querySelector("#save-account-import").addEventListener("click", importAccounts);
}
async function importAccounts(event) {
const form = modal.querySelector("#account-import-form");
if (!form.reportValidity()) return;
const button = event.currentTarget;
const errorBox = modal.querySelector("#account-import-error");
const resultBox = modal.querySelector("#account-import-result");
button.disabled = true;
errorBox.textContent = "";
resultBox.hidden = true;
try {
  const payloads = [];
  for (const file of form.elements.files.files) {
    let parsed;
    try { parsed = JSON.parse(await file.text()); }
    catch { throw new Error(`${file.name} 不是有效的 JSON 文件`); }
    const data = parsed.data?.accounts && parsed.data?.proxies ? parsed.data : parsed;
    if (!Array.isArray(data.accounts) || !Array.isArray(data.proxies)) throw new Error(`${file.name} 缺少 accounts 或 proxies 数组`);
    if (data.type && !["sub2api-data", "sub2api-bundle"].includes(data.type)) throw new Error(`${file.name} 的备份类型不受支持`);
    if (data.version && data.version !== 1) throw new Error(`${file.name} 的备份版本不受支持`);
    payloads.push(data);
  }
  const merged = {
    type: "sub2api-data",
    version: 1,
    exported_at: new Date().toISOString(),
    proxies: payloads.flatMap(item => item.proxies),
    accounts: payloads.flatMap(item => item.accounts),
  };
  const response = await api("/api/admin/accounts/data", { method: "POST", body: JSON.stringify({ data: merged, skip_default_group_bind: true }) });
  const result = response.data;
  const errors = result.errors || [];
  resultBox.innerHTML = `<div class="import-summary"><span>代理新建 <strong>${result.proxy_created}</strong></span><span>代理复用 <strong>${result.proxy_reused}</strong></span><span>账号新建 <strong>${result.account_created}</strong></span><span>失败 <strong>${result.proxy_failed + result.account_failed}</strong></span></div>${errors.length ? `<div class="import-errors">${errors.map(item => `<div><strong>${item.kind === "proxy" ? "代理" : "账号"} · ${escapeHtml(item.name || item.proxy_key || "-")}</strong><span>${escapeHtml(item.message)}</span></div>`).join("")}</div>` : ""}`;
  resultBox.hidden = false;
  toast(errors.length ? "导入完成，部分条目未导入" : "账号备份已导入", errors.length > 0);
  button.textContent = "再次导入";
  await renderRoute();
} catch (error) {
  errorBox.textContent = error.message || "导入失败";
} finally { button.disabled = false; }
}
async function exportAccounts(ids = []) {
if (!confirm("导出的 JSON 包含上游密钥、OAuth Token 和代理密码。确认下载敏感备份？")) return;
try {
  const query = ids.length ? `?ids=${ids.map(Number).join(",")}` : "";
  const result = await api(`/api/admin/accounts/data${query}`);
  const blob = new Blob([JSON.stringify(result.data, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `sub2api-data-${new Date().toISOString().slice(0, 10)}.json`;
  link.click();
  URL.revokeObjectURL(url);
  toast(ids.length ? `已导出 ${result.data.accounts.length} 个选中账号` : "敏感账号备份已导出");
} catch (error) { toast(error.message, true); }
}
async function renderProxies(page) {
const result = await api("/api/admin/proxies");
currentProxies = result.data;
selectedProxyIds = new Set();
page.innerHTML = `${pageHeader("网络代理", `${result.data.length} 个代理`, `<button class="button danger" id="delete-selected-proxies" hidden>删除所选</button><button class="button secondary" id="import-proxies">导入</button><button class="button secondary" id="export-proxies">导出</button><button class="button" id="add-proxy">添加代理</button>`)}
  ${result.data.length ? `<div class="inline-filters"><div class="field"><label for="proxy-search">搜索</label><input id="proxy-search" placeholder="名称、地址或出口 IP"></div><div class="field"><label for="proxy-status-filter">状态</label><select id="proxy-status-filter"><option value="">全部</option><option value="active">可用</option><option value="inactive">停用</option><option value="expired">过期</option></select></div><div class="field"><label for="proxy-protocol-filter">协议</label><select id="proxy-protocol-filter"><option value="">全部</option><option value="http">HTTP</option><option value="https">HTTPS</option><option value="socks5">SOCKS5</option><option value="socks5h">SOCKS5H</option></select></div></div><div id="proxy-list"></div>` : emptyState("暂无网络代理", "添加后可绑定到指定上游账号", "添加代理", "empty-add-proxy")}`;
page.querySelector("#add-proxy")?.addEventListener("click", () => openProxyModal());
page.querySelector("#empty-add-proxy")?.addEventListener("click", () => openProxyModal());
page.querySelector("#import-proxies")?.addEventListener("click", openProxyImportModal);
page.querySelector("#export-proxies")?.addEventListener("click", exportProxies);
page.querySelector("#delete-selected-proxies")?.addEventListener("click", confirmBatchDeleteProxies);
["#proxy-search", "#proxy-status-filter", "#proxy-protocol-filter"].forEach(selector => page.querySelector(selector)?.addEventListener("input", () => updateProxyList(page)));
updateProxyList(page);
}
function proxyTable(proxies) {
return `<div class="table-wrap"><table><thead><tr><th><input type="checkbox" id="proxy-select-all" aria-label="选择当前代理"></th><th>名称</th><th>地址</th><th>状态</th><th>出口与质量</th><th>回退</th><th>账号</th><th></th></tr></thead><tbody>
  ${proxies.map(proxy => `<tr><td><input type="checkbox" data-proxy-select value="${proxy.id}" aria-label="选择 ${escapeHtml(proxy.name)}" ${selectedProxyIds.has(proxy.id) ? "checked" : ""}></td><td><span class="cell-main">${escapeHtml(proxy.name)}</span><span class="cell-sub mono">${escapeHtml(proxy.protocol.toUpperCase())}</span></td><td><span class="mono">${escapeHtml(proxy.address)}</span>${proxy.username ? `<span class="cell-sub">用户 ${escapeHtml(proxy.username)}</span>` : ""}</td><td>${proxy.status === "active" ? status("可用") : proxy.status === "expired" ? status("已过期", "warn") : status("停用", "off")}${proxy.last_error ? `<span class="cell-sub">${escapeHtml(proxy.last_error)}</span>` : ""}</td><td>${proxy.ip_address ? `<span class="cell-main mono">${escapeHtml(proxy.ip_address)}</span><span class="cell-sub">${escapeHtml([proxy.country, proxy.region, proxy.city].filter(Boolean).join(" · "))}</span>` : `<span class="cell-sub">尚未检测出口</span>`}${proxy.quality_score == null ? "" : `<span class="cell-sub">质量 ${proxy.quality_grade} · ${proxy.quality_score} 分</span>`}</td><td>${proxy.fallback_mode === "proxy" ? `<span class="cell-main">备用代理</span><span class="cell-sub">${escapeHtml(proxy.backup_proxy_name || "未配置")}</span>` : proxy.fallback_mode === "direct" ? `<span class="cell-main">故障时直连</span>` : `<span class="cell-sub">不回退</span>`}</td><td>${proxy.account_count}</td><td><div class="cell-actions"><button class="button quiet small" data-proxy-action="test" data-id="${proxy.id}">测试</button><button class="button quiet small" data-proxy-action="quality" data-id="${proxy.id}">质量</button><button class="button quiet small" data-proxy-action="stats" data-id="${proxy.id}">统计</button><button class="button quiet small" data-proxy-action="edit" data-id="${proxy.id}">编辑</button><button class="button quiet small" data-proxy-action="toggle" data-id="${proxy.id}" data-enabled="${proxy.enabled}">${proxy.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-proxy-action="delete" data-id="${proxy.id}">删除</button></div></td></tr>`).join("")}
</tbody></table></div>`;
}
function updateProxyList(page) {
const search = page.querySelector("#proxy-search")?.value.trim().toLowerCase() || "";
const statusFilter = page.querySelector("#proxy-status-filter")?.value || "";
const protocol = page.querySelector("#proxy-protocol-filter")?.value || "";
const visible = currentProxies.filter(proxy => {
  const haystack = [proxy.name, proxy.address, proxy.ip_address, proxy.country, proxy.region, proxy.city].filter(Boolean).join(" ").toLowerCase();
  return (!search || haystack.includes(search)) && (!statusFilter || proxy.status === statusFilter) && (!protocol || proxy.protocol === protocol);
});
const container = page.querySelector("#proxy-list");
if (!container) return;
container.innerHTML = visible.length ? proxyTable(visible) : emptyState("没有匹配的代理", "调整搜索或筛选条件");
container.querySelectorAll("[data-proxy-action]").forEach(button => button.addEventListener("click", handleProxyAction));
container.querySelectorAll("[data-proxy-select]").forEach(input => input.addEventListener("change", event => {
  const id = Number(event.target.value);
  event.target.checked ? selectedProxyIds.add(id) : selectedProxyIds.delete(id);
  updateProxyBulkButton(page);
}));
const selectAll = container.querySelector("#proxy-select-all");
if (selectAll) {
  selectAll.checked = visible.length > 0 && visible.every(proxy => selectedProxyIds.has(proxy.id));
  selectAll.addEventListener("change", event => {
    visible.forEach(proxy => event.target.checked ? selectedProxyIds.add(proxy.id) : selectedProxyIds.delete(proxy.id));
    updateProxyList(page);
  });
}
updateProxyBulkButton(page);
}
function updateProxyBulkButton(page) {
const button = page.querySelector("#delete-selected-proxies");
if (!button) return;
button.hidden = selectedProxyIds.size === 0;
button.textContent = `删除所选 (${selectedProxyIds.size})`;
}
function openProxyModal(proxy = null) {
const backupOptions = currentProxies.filter(item => item.id !== proxy?.id).map(item => `<option value="${item.id}" ${String(item.id) === String(proxy?.backup_proxy_id) ? "selected" : ""}>${escapeHtml(item.name)} · ${escapeHtml(item.address)}</option>`).join("");
openModal(proxy ? "编辑网络代理" : "添加网络代理", `<form id="proxy-form">
  <div class="field"><label for="proxy-name">名称</label><input id="proxy-name" name="name" value="${escapeHtml(proxy?.name || "")}" maxlength="80" placeholder="办公网络" required autofocus></div>
  <div class="field"><label for="proxy-url">代理 URL</label><input id="proxy-url" name="url" type="password" placeholder="http://user:password@host:3128" ${proxy ? "" : "required"} autocomplete="new-password"><span class="field-hint">支持 HTTP、HTTPS、SOCKS5 和 SOCKS5H；${proxy ? "留空保留现有地址" : "用户名和密码会加密保存"}</span></div>
  <div class="field"><label for="proxy-expiry">到期时间</label><input id="proxy-expiry" name="expires_at" type="datetime-local" value="${toDateTimeLocal(proxy?.expires_at)}"><span class="field-hint">留空表示永不过期</span></div>
  <div class="form-grid"><div class="field"><label for="proxy-fallback">故障回退</label><select id="proxy-fallback" name="fallback_mode"><option value="none" ${!proxy || proxy.fallback_mode === "none" ? "selected" : ""}>停止调度</option><option value="proxy" ${proxy?.fallback_mode === "proxy" ? "selected" : ""}>备用代理</option><option value="direct" ${proxy?.fallback_mode === "direct" ? "selected" : ""}>直接连接</option></select></div><div class="field"><label for="proxy-warn-days">到期提醒天数</label><input id="proxy-warn-days" name="expiry_warn_days" type="number" min="0" max="3650" value="${proxy?.expiry_warn_days ?? 7}" required></div></div>
  <div class="field" id="proxy-backup-field" ${proxy?.fallback_mode === "proxy" ? "" : "hidden"}><label for="proxy-backup">备用代理</label><select id="proxy-backup" name="backup_proxy_id"><option value="">请选择</option>${backupOptions}</select></div>
  <label class="switch-row"><span><strong>启用代理</strong><small>停用或过期后按故障回退策略处理</small></span><input name="enabled" type="checkbox" ${proxy?.enabled === false ? "" : "checked"}></label>
  <p class="form-error" id="proxy-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-proxy">保存</button>`);
modal.querySelector("#save-proxy").addEventListener("click", () => saveProxy(proxy?.id));
modal.querySelector("#proxy-fallback").addEventListener("change", event => {
  modal.querySelector("#proxy-backup-field").hidden = event.target.value !== "proxy";
});
}
async function saveProxy(id = null) {
const form = modal.querySelector("#proxy-form");
if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
values.enabled = form.elements.enabled.checked;
values.expires_at = values.expires_at ? new Date(values.expires_at).toISOString() : null;
values.expiry_warn_days = Number(values.expiry_warn_days);
values.backup_proxy_id = values.fallback_mode === "proxy" && values.backup_proxy_id ? Number(values.backup_proxy_id) : null;
if (id && !values.url) delete values.url;
const button = modal.querySelector("#save-proxy");
button.disabled = true;
try {
  await api(id ? `/api/admin/proxies/${id}` : "/api/admin/proxies", { method: id ? "PUT" : "POST", body: JSON.stringify(values) });
  closeModal(); toast(id ? "代理设置已更新" : "代理已添加"); await renderRoute();
} catch (error) { modal.querySelector("#proxy-error").textContent = error.message; }
finally { button.disabled = false; }
}
async function handleProxyAction(event) {
const button = event.currentTarget;
const proxy = currentProxies.find(item => String(item.id) === String(button.dataset.id));
if (!proxy) return;
const action = button.dataset.proxyAction;
if (action === "edit") return openProxyModal(proxy);
if (action === "delete") {
  openModal("删除网络代理", `<p>确认删除 <strong>${escapeHtml(proxy.name)}</strong>？已绑定账号时需要先解除绑定。</p><p class="form-error" id="proxy-delete-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-delete-proxy">删除</button>`);
  modal.querySelector("#confirm-delete-proxy").addEventListener("click", async event => {
    event.currentTarget.disabled = true;
    try { await api(`/api/admin/proxies/${proxy.id}`, { method: "DELETE" }); closeModal(); toast("代理已删除"); await renderRoute(); }
    catch (error) { modal.querySelector("#proxy-delete-error").textContent = error.message; event.currentTarget.disabled = false; }
  });
  return;
}
button.disabled = true;
try {
  if (action === "test") {
    const result = await api(`/api/admin/proxies/${proxy.id}/test`, { method: "POST", body: "{}" });
    toast(result.data.success ? `代理连通，延迟 ${result.data.latency_ms} ms${result.data.ip_address ? `，出口 ${result.data.ip_address}` : ""}` : result.data.message, !result.data.success);
  } else if (action === "quality") {
    const result = await api(`/api/admin/proxies/${proxy.id}/quality-check`, { method: "POST", body: "{}" });
    openModal(`${proxy.name} · 质量 ${result.data.grade}`, `<div class="metrics">${metric("评分", result.data.score)}${metric("通过", result.data.passed_count, "good")}${metric("警告", result.data.warn_count, result.data.warn_count ? "warn" : "")}${metric("失败", result.data.failed_count, result.data.failed_count ? "warn" : "")}</div><div class="table-wrap"><table><thead><tr><th>目标</th><th>结果</th><th>HTTP</th><th>延迟</th></tr></thead><tbody>${result.data.items.map(item => `<tr><td>${escapeHtml(item.target)}</td><td>${item.status === "pass" ? status("通过") : item.status === "challenge" ? status("验证挑战", "warn") : item.status === "warn" ? status("警告", "warn") : status("失败", "off")}</td><td>${item.http_status || "-"}</td><td>${item.latency_ms == null ? "-" : `${item.latency_ms} ms`}</td></tr>`).join("")}</tbody></table></div>`, `<button class="button" data-close-modal>关闭</button>`);
    return;
  } else if (action === "stats") {
    const [statsResult, accountsResult] = await Promise.all([api(`/api/admin/proxies/${proxy.id}/stats`), api(`/api/admin/proxies/${proxy.id}/accounts`)]);
    const stats = statsResult.data;
    openModal(`${proxy.name} · 使用统计`, `<div class="metrics">${metric("账号", stats.total_accounts)}${metric("启用账号", stats.active_accounts, "good")}${metric("请求", stats.total_requests)}${metric("成功率", `${Number(stats.success_rate).toFixed(1)}%`)}${metric("平均耗时", `${stats.average_latency} ms`)}</div>${accountsResult.data.length ? `<div class="table-wrap"><table><thead><tr><th>账号</th><th>类型</th><th>状态</th></tr></thead><tbody>${accountsResult.data.map(account => `<tr><td>${escapeHtml(account.name)}</td><td>${account.kind === "oauth" ? "OAuth" : "API Key"}</td><td>${account.enabled ? status("启用") : status("停用", "off")}</td></tr>`).join("")}</tbody></table></div>` : emptyState("没有绑定账号", "")}`, `<button class="button" data-close-modal>关闭</button>`);
    return;
  } else if (action === "toggle") {
    await api(`/api/admin/proxies/${proxy.id}`, { method: "PUT", body: JSON.stringify({ enabled: !proxy.enabled }) });
    toast("代理状态已更新");
  }
  await renderRoute();
} catch (error) { toast(error.message, true); }
finally { button.disabled = false; }
}
function confirmBatchDeleteProxies() {
const ids = [...selectedProxyIds];
if (!ids.length) return;
openModal("批量删除网络代理", `<p>将删除所选的 ${ids.length} 个代理；仍被账号或备用链引用的代理会自动跳过。</p><p class="form-error" id="proxy-batch-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-batch-delete-proxies">删除</button>`);
modal.querySelector("#confirm-batch-delete-proxies").addEventListener("click", async event => {
  event.currentTarget.disabled = true;
  try {
    const result = await api("/api/admin/proxies/batch-delete", { method: "POST", body: JSON.stringify({ ids }) });
    closeModal(); toast(`已删除 ${result.data.deleted_ids.length} 个，跳过 ${result.data.skipped.length} 个`); await renderRoute();
  } catch (error) { modal.querySelector("#proxy-batch-error").textContent = error.message; event.currentTarget.disabled = false; }
});
}
function openProxyImportModal() {
openModal("导入网络代理", `<form id="proxy-import-form"><div class="field"><label for="proxy-import-data">JSON 数据</label><textarea id="proxy-import-data" name="data" spellcheck="false" placeholder='{"proxies":[{"name":"proxy","url":"http://host:3128"}]}' required autofocus></textarea></div><p class="form-error" id="proxy-import-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-proxy-import">导入</button>`);
modal.querySelector("#save-proxy-import").addEventListener("click", async event => {
  const form = modal.querySelector("#proxy-import-form");
  if (!form.reportValidity()) return;
  event.currentTarget.disabled = true;
  try {
    const parsed = JSON.parse(form.elements.data.value);
    const payload = Array.isArray(parsed) ? { proxies: parsed } : parsed.data?.proxies ? { proxies: parsed.data.proxies } : parsed;
    const result = await api("/api/admin/proxies/data", { method: "POST", body: JSON.stringify(payload) });
    closeModal(); toast(`已导入 ${result.data.created} 个，跳过 ${result.data.skipped} 个`); await renderRoute();
  } catch (error) { modal.querySelector("#proxy-import-error").textContent = error.message || "JSON 格式无效"; event.currentTarget.disabled = false; }
});
}
async function exportProxies() {
try {
  const result = await api("/api/admin/proxies/data");
  const blob = new Blob([JSON.stringify(result.data, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url; link.download = `sub2api-mini-proxies-${new Date().toISOString().slice(0, 10)}.json`; link.click();
  URL.revokeObjectURL(url); toast("代理数据已导出");
} catch (error) { toast(error.message, true); }
}
async function renderKeys(page) {
const requests = [api(`${roleApiBase()}/keys`), api(`${roleApiBase()}/groups`)];
if (state.role === "admin") requests.push(api("/api/admin/users"));
const [result, groups, owners] = await Promise.all(requests);
currentKeys = result.data;
currentGroups = groups.data.filter(group => group.enabled !== false);
currentKeyOwners = owners?.data || [];
selectedKeyIds = new Set();
page.innerHTML = `
  ${pageHeader("API Key", `${result.data.length} 个下游密钥`, `<button class="button" id="add-key">创建 Key</button>`)}
  ${result.data.length ? `<div class="inline-filters key-filters"><div class="field"><label for="key-search">搜索</label><input id="key-search" type="search" placeholder="名称、前缀、所有者或 IP"></div><div class="field"><label for="key-status-filter">状态</label><select id="key-status-filter"><option value="">全部</option><option value="active">有效</option><option value="inactive">停用</option><option value="expired">过期</option><option value="quota_exhausted">额度耗尽</option></select></div><div class="field"><label for="key-group-filter">路由分组</label><select id="key-group-filter"><option value="">全部</option><option value="0">未分组</option>${currentGroups.map(group => `<option value="${group.id}">${escapeHtml(group.name)}</option>`).join("")}</select></div></div><div class="key-batch-bar"><span id="key-selection-count">未选择 Key</span><select id="key-batch-action" aria-label="批量操作"><option value="enable">启用</option><option value="disable">停用</option><option value="reset_quota">重置总额度用量</option><option value="reset_rate_limit">重置窗口用量</option><option value="delete">撤销</option></select><button class="button secondary" id="apply-key-batch" disabled>应用</button></div><div id="key-list"></div>` : emptyState("暂无 API Key", "创建后可用于访问网关接口", "创建 Key", "empty-add-key")}`;
document.querySelector("#add-key")?.addEventListener("click", openKeyModal);
document.querySelector("#empty-add-key")?.addEventListener("click", openKeyModal);
["#key-search", "#key-status-filter", "#key-group-filter"].forEach(selector => page.querySelector(selector)?.addEventListener("input", () => updateKeyList(page)));
page.querySelector("#apply-key-batch")?.addEventListener("click", applyKeyBatch);
if (result.data.length) updateKeyList(page);
}
function keyTable(keys) {
const ownerHeader = state.role === "admin" ? "<th>所有者</th>" : "";
return `<div class="table-wrap"><table><thead><tr><th><input type="checkbox" id="key-select-all" aria-label="选择当前 Key"></th><th>名称</th>${ownerHeader}<th>分组</th><th>前缀</th><th>状态</th><th>额度与窗口</th><th>网络策略</th><th class="hide-mobile">最后使用</th><th>创建时间</th><th></th></tr></thead>
  <tbody>${keys.map(key => `<tr>
    <td><input type="checkbox" data-key-select value="${key.id}" aria-label="选择 ${escapeHtml(key.name)}" ${selectedKeyIds.has(key.id) ? "checked" : ""}></td><td class="cell-main">${escapeHtml(key.name)}</td>${state.role === "admin" ? `<td class="mono">${escapeHtml(key.owner_username || "system")}</td>` : ""}<td>${key.group_name ? status(key.group_name) : `<span class="cell-sub">全部账号</span>`}</td><td class="mono">${escapeHtml(key.token_prefix)}...</td>
    <td>${key.status === "expired" ? status("已过期", "error") : key.status === "quota_exhausted" ? status("额度耗尽", "warn") : key.enabled ? status("有效") : status("停用", "off")}</td>
    <td><span class="cell-main">${key.quota_tokens ? `${formatNumber(key.used_tokens)} / ${formatNumber(key.quota_tokens)} Token` : "无限 Token"}</span><span class="cell-sub">${key.quota_cost_microusd ? `${formatMicrousd(key.used_cost_microusd)} / ${formatMicrousd(key.quota_cost_microusd)}` : "无限总消费"}</span><span class="cell-sub">5h ${limitProgress(key.usage_5h_microusd, key.rate_limit_5h_microusd)} · 1d ${limitProgress(key.usage_1d_microusd, key.rate_limit_1d_microusd)} · 7d ${limitProgress(key.usage_7d_microusd, key.rate_limit_7d_microusd)}</span><span class="cell-sub">${key.expires_at ? `到期 ${formatDate(key.expires_at)}` : "永不过期"}${key.allowed_models?.length ? ` · ${key.allowed_models.length} 个模型` : " · 所有模型"}</span></td>
    <td><span class="cell-main">白名单 ${key.ip_whitelist?.length || 0} · 黑名单 ${key.ip_blacklist?.length || 0}</span><span class="cell-sub mono">${escapeHtml(key.last_used_ip || "尚无来源 IP")}</span></td><td class="hide-mobile">${formatDate(key.last_used_at)}</td><td>${formatDate(key.created_at)}</td>
    <td><div class="cell-actions"><button class="button quiet small" data-key-action="edit" data-id="${key.id}">编辑</button><button class="button quiet small" data-key-action="toggle" data-id="${key.id}" data-enabled="${key.enabled}">${key.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-key-action="delete" data-id="${key.id}">撤销</button></div></td>
  </tr>`).join("")}</tbody></table></div>`;
}
function updateKeyList(page) {
const query = page.querySelector("#key-search")?.value.trim().toLowerCase() || "";
const stateFilter = page.querySelector("#key-status-filter")?.value || "";
const groupFilter = page.querySelector("#key-group-filter")?.value || "";
const visible = currentKeys.filter(key => {
  const searchable = [key.name, key.token_prefix, key.owner_username, key.last_used_ip, ...(key.ip_whitelist || []), ...(key.ip_blacklist || [])].filter(Boolean).join(" ").toLowerCase();
  return (!query || searchable.includes(query)) && (!stateFilter || key.status === stateFilter) && (!groupFilter || String(key.group_id || 0) === groupFilter);
});
const container = page.querySelector("#key-list");
if (!container) return;
container.innerHTML = visible.length ? keyTable(visible) : emptyState("没有匹配的 Key", "调整搜索或筛选条件");
container.querySelectorAll("[data-key-action]").forEach(button => button.addEventListener("click", handleKeyAction));
container.querySelectorAll("[data-key-select]").forEach(input => input.addEventListener("change", event => {
  const id = Number(event.currentTarget.value);
  event.currentTarget.checked ? selectedKeyIds.add(id) : selectedKeyIds.delete(id);
  updateKeyBatchState(page, visible);
}));
const selectAll = container.querySelector("#key-select-all");
if (selectAll) {
  selectAll.checked = visible.length > 0 && visible.every(key => selectedKeyIds.has(key.id));
  selectAll.addEventListener("change", event => {
    visible.forEach(key => event.currentTarget.checked ? selectedKeyIds.add(key.id) : selectedKeyIds.delete(key.id));
    updateKeyList(page);
  });
}
updateKeyBatchState(page, visible);
}
function updateKeyBatchState(page, visible = currentKeys) {
const count = page.querySelector("#key-selection-count");
const button = page.querySelector("#apply-key-batch");
if (count) count.textContent = selectedKeyIds.size ? `已选择 ${selectedKeyIds.size} 个 Key` : "未选择 Key";
if (button) button.disabled = selectedKeyIds.size === 0;
const selectAll = page.querySelector("#key-select-all");
if (selectAll) selectAll.checked = visible.length > 0 && visible.every(key => selectedKeyIds.has(key.id));
}
async function applyKeyBatch() {
const action = document.querySelector("#key-batch-action")?.value;
const labels = { enable: "启用", disable: "停用", reset_quota: "重置总额度用量", reset_rate_limit: "重置窗口用量", delete: "撤销" };
const ids = [...selectedKeyIds];
if (!ids.length || !action) return;
if (["delete", "reset_quota", "reset_rate_limit"].includes(action) && !confirm(`确认${labels[action]}所选 ${ids.length} 个 Key？`)) return;
const button = document.querySelector("#apply-key-batch");
button.disabled = true;
try {
  const result = await api(`${roleApiBase()}/keys/batch`, { method: "POST", body: JSON.stringify({ ids, action }) });
  toast(`${labels[action]}完成，共处理 ${result.data.affected} 个 Key`);
  await renderRoute();
} catch (error) { toast(error.message, true); button.disabled = false; }
}
function openKeyModal() {
openModal("创建 API Key", `<form id="key-form">
  <div class="field"><label for="key-name">名称</label><input id="key-name" name="name" placeholder="Codex 客户端" maxlength="80" required autofocus></div>
  <div class="field"><label for="key-custom">自定义 Key（可选）</label><input id="key-custom" name="custom_key" type="password" minlength="20" maxlength="200" pattern="sk-[A-Za-z0-9_-]{17,197}" autocomplete="new-password" placeholder="留空自动生成 sk-mini_..."><span class="field-hint">必须以 sk- 开头；完整值仍只在创建时显示一次</span></div>
  ${state.role === "admin" ? `<div class="field"><label for="key-owner">所有者</label><select id="key-owner" name="user_id">${currentKeyOwners.filter(user => user.enabled).map(user => `<option value="${user.id}" ${user.username === state.user ? "selected" : ""}>${escapeHtml(user.display_name || user.username)} · ${escapeHtml(user.username)}</option>`).join("")}</select></div>` : ""}
  <div class="field"><label for="key-group">路由分组</label><select id="key-group" name="group_id">${groupOptions()}</select><span class="field-hint">未分组时可使用全部启用账号</span></div>
  ${keyPolicyFields(null, "key")}
  <div class="field"><label for="key-models">允许的模型</label><textarea id="key-models" name="allowed_models" class="compact-textarea" placeholder="每行一个模型 ID；留空允许全部"></textarea></div>
  <p class="form-error" id="key-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-key">创建</button>`);
modal.querySelector("#save-key").addEventListener("click", saveKey);
}
async function saveKey() {
const form = modal.querySelector("#key-form");
if (!form.reportValidity()) return;
const button = modal.querySelector("#save-key");
button.disabled = true;
try {
  const values = collectKeyPolicy(form, true);
  const result = await api(`${roleApiBase()}/keys`, { method: "POST", body: JSON.stringify(values) });
  openModal("API Key 已创建", `<p>完整密钥仅显示一次。</p><div class="secret-box mono" id="created-token">${escapeHtml(result.data.token)}</div>`, `<button class="button secondary" id="copy-token">复制</button><button class="button" id="finish-key">完成</button>`);
  modal.querySelector("#copy-token").addEventListener("click", async () => { await navigator.clipboard.writeText(result.data.token); toast("已复制"); });
  modal.querySelector("#finish-key").addEventListener("click", async () => { closeModal(); await renderRoute(); });
} catch (error) { modal.querySelector("#key-error").textContent = error.message; }
finally { button.disabled = false; }
}
async function handleKeyAction(event) {
const button = event.currentTarget;
const id = button.dataset.id;
const action = button.dataset.keyAction;
if (action === "edit") {
  const key = currentKeys.find(item => String(item.id) === String(id));
  if (key) openKeyEditModal(key);
  return;
}
if (action === "delete" && !confirm("撤销后使用该 Key 的客户端会立即失效，确认继续？")) return;
try {
  if (action === "toggle") await api(`${roleApiBase()}/keys/${id}`, { method: "PUT", body: JSON.stringify({ enabled: button.dataset.enabled !== "true" }) });
  else await api(`${roleApiBase()}/keys/${id}`, { method: "DELETE" });
  toast(action === "delete" ? "Key 已撤销" : "Key 已更新");
  await renderRoute();
} catch (error) { toast(error.message, true); }
}
function openKeyEditModal(key) {
openModal("编辑 API Key", `<form id="key-edit-form">
  <div class="field"><label for="edit-key-name">名称</label><input id="edit-key-name" name="name" value="${escapeHtml(key.name)}" maxlength="80" required autofocus></div>
  <div class="field"><label for="edit-key-group">路由分组</label><select id="edit-key-group" name="group_id">${groupOptions(key.group_id)}</select></div>
  ${keyPolicyFields(key, "edit-key")}
  <div class="field"><label for="edit-key-models">允许的模型</label><textarea id="edit-key-models" name="allowed_models" class="compact-textarea" placeholder="每行一个模型 ID；留空允许全部">${escapeHtml((key.allowed_models || []).join("\n"))}</textarea></div>
  <div class="form-grid"><label class="switch-row compact"><span><strong>重置总额度</strong><small>保存后从零重新累计 Token 和消费</small></span><input name="reset_quota" type="checkbox"></label><label class="switch-row compact"><span><strong>重置窗口用量</strong><small>保存后重新累计 5h / 1d / 7d</small></span><input name="reset_rate_limit_usage" type="checkbox"></label></div>
  <p class="form-error" id="key-edit-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-key-edit">保存</button>`);
modal.querySelector("#save-key-edit").addEventListener("click", () => saveKeyEdit(key.id));
}
async function saveKeyEdit(id) {
const form = modal.querySelector("#key-edit-form");
if (!form.reportValidity()) return;
const button = modal.querySelector("#save-key-edit");
const values = collectKeyPolicy(form, false);
button.disabled = true;
try {
  await api(`${roleApiBase()}/keys/${id}`, { method: "PUT", body: JSON.stringify(values) });
  closeModal();
  toast("Key 策略已更新");
  await renderRoute();
} catch (error) {
  modal.querySelector("#key-edit-error").textContent = error.message;
} finally {
  button.disabled = false;
}
}
function keyPolicyFields(key, prefix) {
const editing = Boolean(key);
return `<div class="form-grid"><div class="field"><label for="${prefix}-expiry">${editing ? "到期时间" : "有效天数"}</label><input id="${prefix}-expiry" name="${editing ? "expires_at" : "expires_in_days"}" type="${editing ? "datetime-local" : "number"}" ${editing ? `value="${escapeHtml(toDateTimeLocal(key.expires_at))}"` : 'min="0" max="3650" value="0"'}><span class="field-hint">${editing ? "留空表示永不过期" : "0 表示永不过期"}</span></div><div class="field"><label for="${prefix}-quota-token">Token 总配额</label><input id="${prefix}-quota-token" name="quota_tokens" type="number" min="0" max="1000000000000000" value="${Number(key?.quota_tokens || 0)}"><span class="field-hint">${editing ? `已使用 ${formatNumber(key.used_tokens || 0)} Token` : "0 表示无限"}</span></div></div>
  <div class="field"><label for="${prefix}-quota-cost">消费总额度 (USD)</label><input id="${prefix}-quota-cost" name="quota_cost_usd" type="number" min="0" max="1000000000" step="0.000001" value="${microusdInput(key?.quota_cost_microusd)}"><span class="field-hint">${editing ? `已使用 ${formatMicrousd(key.used_cost_microusd)}` : "0 表示无限；数据库以整数微美元保存"}</span></div>
  <div class="pricing-grid key-rate-grid"><div class="field"><label for="${prefix}-rate-5h">5 小时额度 (USD)</label><input id="${prefix}-rate-5h" name="rate_limit_5h_usd" type="number" min="0" max="1000000000" step="0.000001" value="${microusdInput(key?.rate_limit_5h_microusd)}"><span class="field-hint">已用 ${formatMicrousd(key?.usage_5h_microusd)}</span></div><div class="field"><label for="${prefix}-rate-1d">1 日额度 (USD)</label><input id="${prefix}-rate-1d" name="rate_limit_1d_usd" type="number" min="0" max="1000000000" step="0.000001" value="${microusdInput(key?.rate_limit_1d_microusd)}"><span class="field-hint">已用 ${formatMicrousd(key?.usage_1d_microusd)}</span></div><div class="field"><label for="${prefix}-rate-7d">7 日额度 (USD)</label><input id="${prefix}-rate-7d" name="rate_limit_7d_usd" type="number" min="0" max="1000000000" step="0.000001" value="${microusdInput(key?.rate_limit_7d_microusd)}"><span class="field-hint">已用 ${formatMicrousd(key?.usage_7d_microusd)}</span></div></div>
  <div class="form-grid"><div class="field"><label for="${prefix}-whitelist">IP 白名单</label><textarea id="${prefix}-whitelist" name="ip_whitelist" class="compact-textarea" placeholder="每行一个 IP 或 CIDR">${escapeHtml((key?.ip_whitelist || []).join("\n"))}</textarea><span class="field-hint">非空时只允许列表内来源</span></div><div class="field"><label for="${prefix}-blacklist">IP 黑名单</label><textarea id="${prefix}-blacklist" name="ip_blacklist" class="compact-textarea" placeholder="每行一个 IP 或 CIDR">${escapeHtml((key?.ip_blacklist || []).join("\n"))}</textarea><span class="field-hint">黑名单优先于白名单</span></div></div>`;
}
function collectKeyPolicy(form, creating) {
const values = Object.fromEntries(new FormData(form));
values.quota_tokens = Number(values.quota_tokens || 0);
values.quota_cost_microusd = usdToMicrousd(values.quota_cost_usd);
values.rate_limit_5h_microusd = usdToMicrousd(values.rate_limit_5h_usd);
values.rate_limit_1d_microusd = usdToMicrousd(values.rate_limit_1d_usd);
values.rate_limit_7d_microusd = usdToMicrousd(values.rate_limit_7d_usd);
values.group_id = Number(values.group_id || 0);
if (values.user_id) values.user_id = Number(values.user_id);
values.allowed_models = parseModelList(values.allowed_models);
values.ip_whitelist = parseModelList(values.ip_whitelist);
values.ip_blacklist = parseModelList(values.ip_blacklist);
if (creating) values.expires_in_days = Number(values.expires_in_days || 0);
else values.expires_at = values.expires_at ? new Date(values.expires_at).toISOString() : "";
values.reset_quota = Boolean(form.elements.reset_quota?.checked);
values.reset_rate_limit_usage = Boolean(form.elements.reset_rate_limit_usage?.checked);
delete values.quota_cost_usd;
delete values.rate_limit_5h_usd;
delete values.rate_limit_1d_usd;
delete values.rate_limit_7d_usd;
return values;
}
function usdToMicrousd(value) {
const amount = Number(value || 0);
if (!Number.isFinite(amount) || amount < 0 || amount > 1000000000) throw new Error("消费额度必须在 0 到 10 亿美元之间");
return Math.round(amount * 1000000);
}
function microusdInput(value) { return String((Number(value) || 0) / 1000000); }
function formatMicrousd(value) { return `$${((Number(value) || 0) / 1000000).toLocaleString("zh-CN", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`; }
function limitProgress(used, limit) { return Number(limit) > 0 ? `${formatMicrousd(used)} / ${formatMicrousd(limit)}` : "无限"; }
function groupOptions(selected = 0) {
return `<option value="0" ${!selected ? "selected" : ""}>未分组（全部账号）</option>${currentGroups.map(group => `<option value="${group.id}" ${String(selected) === String(group.id) ? "selected" : ""}>${escapeHtml(group.name)}</option>`).join("")}`;
}
async function renderModels(page) {
const result = await api("/api/user/models");
const catalog = result.data;
page.innerHTML = `
  ${pageHeader("可用模型", `${catalog.models.length} 个模型，${catalog.sources.length} 个上游来源`, `<button class="button secondary" id="refresh-models">刷新</button>`)}
  <div class="field model-search"><label for="model-search">搜索模型</label><input id="model-search" type="search" placeholder="输入模型 ID"></div>
  <div id="model-list">${modelCatalog(catalog, "")}</div>`;
page.querySelector("#refresh-models").addEventListener("click", renderRoute);
page.querySelector("#model-search").addEventListener("input", event => {
  page.querySelector("#model-list").innerHTML = modelCatalog(catalog, event.currentTarget.value);
});
}
function modelCatalog(catalog, query) {
const search = query.trim().toLowerCase();
const models = catalog.models.filter(model => model.toLowerCase().includes(search));
return `
  <section class="model-cloud">${models.length ? models.map(model => `<code>${escapeHtml(model)}</code>`).join("") : `<p>没有匹配的模型</p>`}</section>
  <section class="section"><div class="section-title"><h2>上游来源</h2></div>
    ${catalog.sources.length ? `<div class="table-wrap"><table><thead><tr><th>账号</th><th>类型</th><th>状态</th><th>模型</th></tr></thead><tbody>${catalog.sources.map(source => `<tr><td class="cell-main">${escapeHtml(source.name)}</td><td>${source.kind === "oauth" ? "OAuth" : "API Key"}</td><td>${source.status === "available" ? status("可用") : status("不可用", "error")}${source.error ? `<span class="cell-sub">${escapeHtml(source.error)}</span>` : ""}</td><td>${formatNumber(source.models.length)}</td></tr>`).join("")}</tbody></table></div>` : emptyState("暂无上游来源", "管理员添加上游账号后会显示可用模型")}
  </section>`;
}
async function renderAvailableChannels(page) {
const result = await api("/api/user/channels/available");
const channels = result.data;
page.innerHTML = `${pageHeader("可用频道", `${channels.length} 个频道`, `<button class="button secondary" id="refresh-channels">刷新</button>`)}<div class="field model-search"><label for="channel-search">搜索频道</label><input id="channel-search" type="search" placeholder="频道、平台、分组或模型"></div><div id="available-channel-list">${availableChannelMarkup(channels, "")}</div>`;
page.querySelector("#refresh-channels")?.addEventListener("click", renderRoute);
page.querySelector("#channel-search")?.addEventListener("input", event => {
  page.querySelector("#available-channel-list").innerHTML = availableChannelMarkup(channels, event.currentTarget.value);
});
}
function formatRateMultiplier(value) {
return `${Number(value == null ? 1 : value).toLocaleString("zh-CN", { maximumFractionDigits: 6 })}x`;
}
function peakRateText(group) {
if (!group.peak_rate_enabled || !group.peak_start || !group.peak_end) return "";
return `${group.peak_start}-${group.peak_end} · ${formatRateMultiplier(group.peak_rate_multiplier)} · UTC${group.server_utc_offset || "+08:00"}`;
}
function groupRateMarkup(group) {
const custom = group.user_rate_multiplier != null;
const peak = peakRateText(group);
return `<span class="rate-pill ${custom ? "custom" : ""}" title="默认 ${formatRateMultiplier(group.rate_multiplier)}${custom ? `，专属 ${formatRateMultiplier(group.user_rate_multiplier)}` : ""}">${custom ? "专属" : "倍率"} ${formatRateMultiplier(group.resolved_rate_multiplier ?? group.rate_multiplier)}</span>${Number(group.applied_peak_multiplier || 1) !== 1 ? `<span class="rate-pill peak" title="${escapeHtml(peak)}">当前 ${formatRateMultiplier(group.effective_rate_multiplier)}</span>` : ""}${peak ? `<span class="peak-window">${escapeHtml(peak)}</span>` : ""}`;
}
function availableChannelMarkup(channels, query) {
const search = query.trim().toLowerCase();
const visible = channels.map(channel => {
  const channelHit = `${channel.name} ${channel.description || ""}`.toLowerCase().includes(search);
  const platforms = (channel.platforms || []).filter(platform => channelHit || !search || `${platform.platform} ${platform.platform_label || ""} ${(platform.groups || []).map(group => group.name).join(" ")} ${(platform.supported_models || []).map(model => model.name).join(" ")}`.toLowerCase().includes(search));
  return platforms.length ? { ...channel, platforms } : null;
}).filter(Boolean);
if (!visible.length) return emptyState(search ? "没有匹配的频道" : "暂无可用频道", search ? "调整搜索条件后重试" : "管理员配置频道与模型价格后会显示");
return `<div class="channel-list">${visible.map(channel => `<section class="channel-band"><div class="section-title"><div><h2>${escapeHtml(channel.name)}</h2><p>${escapeHtml(channel.description || "")}</p></div><span class="cell-sub">${channel.platforms.length} 个平台</span></div>${channel.platforms.map(platform => `<div class="channel-platform"><header class="platform-heading"><div><span class="platform-pill">${escapeHtml(platform.platform_label || platform.platform.toUpperCase())}</span><span class="platform-category">${escapeHtml(platform.platform_category || "custom")}</span></div><span>${platform.model_count || 0} 个模型</span></header><div class="channel-group-rates">${(platform.groups || []).length ? platform.groups.map(group => `<div class="channel-group-rate"><strong>${escapeHtml(group.name)}</strong><span class="type-pill">${group.subscription_type === "subscription" ? "订阅" : "标准"}</span>${groupRateMarkup({ ...group, server_utc_offset: channel.server_utc_offset })}</div>`).join("") : `<span class="cell-sub">此平台未绑定分组</span>`}</div>${availableModelsTable(platform.supported_models || [])}</div>`).join("")}</section>`).join("")}</div>`;
}
function availableModelsTable(models) {
if (!models.length) return `<p class="cell-sub channel-no-models">暂无定价模型</p>`;
return `<div class="table-wrap"><table><thead><tr><th>模型</th><th>计费 / 区间</th><th>输入价</th><th>缓存读 / 写</th><th>图片入 / 出</th><th>输出价</th><th>单次价</th></tr></thead><tbody>${models.map(model => { const pricing = model.pricing || {}; const input = pricing.input_price ?? 0; const output = pricing.output_price ?? 0; return `<tr><td><code>${escapeHtml(model.name)}</code></td><td>${pricing.billing_mode === "request" ? "按请求" : "按 Token"}${pricingIntervalSummary(pricing)}</td><td>${pricing.billing_mode === "tokens" ? `$${Number(input).toFixed(4)} / 1M` : "-"}</td><td>${pricing.billing_mode === "tokens" ? `$${Number(pricing.cache_read_price ?? input).toFixed(4)} / $${Number(pricing.cache_write_price ?? input).toFixed(4)}${pricing.cache_read_price == null || pricing.cache_write_price == null ? `<span class="cell-sub">空值回退输入价</span>` : ""}` : "-"}</td><td>${pricing.billing_mode === "tokens" ? `$${Number(pricing.image_input_price ?? input).toFixed(4)} / $${Number(pricing.image_output_price ?? output).toFixed(4)}${pricing.image_input_price == null || pricing.image_output_price == null ? `<span class="cell-sub">空值回退文本价</span>` : ""}` : "-"}</td><td>${pricing.billing_mode === "tokens" ? `$${Number(output).toFixed(4)} / 1M` : "-"}</td><td>${pricing.billing_mode === "request" ? `$${Number(pricing.per_request_price || 0).toFixed(6)}` : "-"}</td></tr>`; }).join("")}</tbody></table></div>`;
}
function pricingIntervalSummary(pricing) {
const intervals = pricing.intervals || [];
if (!intervals.length) return `<span class="cell-sub">平铺价格</span>`;
return `<span class="cell-sub">${intervals.map(interval => `${formatNumber(interval.min_tokens)}-${interval.max_tokens == null ? "不限" : formatNumber(interval.max_tokens)}`).join(" / ")}</span>`;
}
async function renderChannelAdmin(page) {
const [channels, groups] = await Promise.all([api("/api/admin/channels"), api("/api/admin/groups")]);
currentChannels = channels.data; currentGroups = groups.data;
page.innerHTML = `${pageHeader("频道定价", `${currentChannels.length} 个频道`, `<button class="button" id="add-channel">创建频道</button>`)}${currentChannels.length ? channelAdminTable(currentChannels) : emptyState("暂无频道", "创建频道并绑定路由分组", "创建频道", "empty-add-channel")}`;
page.querySelector("#add-channel")?.addEventListener("click", () => openChannelModal());
page.querySelector("#empty-add-channel")?.addEventListener("click", () => openChannelModal());
page.querySelectorAll("[data-channel-action]").forEach(button => button.addEventListener("click", handleChannelAction));
}
function channelAdminTable(channels) {
return `<div class="table-wrap"><table><thead><tr><th>频道</th><th>状态</th><th>分组</th><th>价格规则</th><th>模型限制</th><th></th></tr></thead><tbody>${channels.map(channel => `<tr><td><span class="cell-main">${escapeHtml(channel.name)}</span><span class="cell-sub">${escapeHtml(channel.description || "")}</span></td><td>${channel.status === "active" ? status("启用") : status("停用", "off")}</td><td>${channel.group_ids.length}</td><td>${channel.model_pricing.length}</td><td>${channel.restrict_models ? status("仅定价模型") : `<span class="cell-sub">不限制</span>`}</td><td><div class="cell-actions"><button class="button quiet small" data-channel-action="edit" data-id="${channel.id}">编辑</button><button class="button quiet small" data-channel-action="toggle" data-id="${channel.id}">${channel.status === "active" ? "停用" : "启用"}</button><button class="button quiet small" data-channel-action="delete" data-id="${channel.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
}
function openChannelModal(channel = null) {
const selectedGroups = new Set(channel?.group_ids || []);
openModal(channel ? "编辑频道" : "创建频道", `<form id="channel-form"><div class="field"><label for="channel-name">名称</label><input id="channel-name" name="name" value="${escapeHtml(channel?.name || "")}" maxlength="100" required autofocus></div><div class="field"><label for="channel-description">说明</label><textarea id="channel-description" name="description" class="compact-textarea">${escapeHtml(channel?.description || "")}</textarea></div><div class="form-grid"><div class="field"><label for="channel-status">状态</label><select id="channel-status" name="status"><option value="active" ${channel?.status !== "inactive" ? "selected" : ""}>启用</option><option value="inactive" ${channel?.status === "inactive" ? "selected" : ""}>停用</option></select></div><label class="switch-row compact"><span><strong>限制模型</strong><small>仅允许价格规则中的模型</small></span><input name="restrict_models" type="checkbox" ${channel?.restrict_models ? "checked" : ""}></label></div><div class="field"><label>路由分组</label><div class="choice-grid">${currentGroups.map(group => `<label><input type="checkbox" name="group_ids" value="${group.id}" ${selectedGroups.has(group.id) ? "checked" : ""}><span>${escapeHtml(group.name)}</span><small>${group.account_ids.length} 个账号</small></label>`).join("") || `<span class="field-hint">暂无路由分组</span>`}</div></div><div class="section-title"><h2>模型价格</h2><button class="button secondary small" type="button" id="add-pricing-rule">添加规则</button></div><div id="pricing-rules"></div><p class="form-error" id="channel-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-channel">保存</button>`);
const rules = channel?.model_pricing?.length ? channel.model_pricing : [{ platform: "openai", models: [], billing_mode: "tokens", input_price: 0, output_price: 0, per_request_price: 0 }];
rules.forEach(rule => addPricingRule(rule));
modal.querySelector("#add-pricing-rule").addEventListener("click", () => addPricingRule());
modal.querySelector("#save-channel").addEventListener("click", () => saveChannel(channel?.id));
}
function addPricingRule(rule = {}) {
const container = modal.querySelector("#pricing-rules");
const row = document.createElement("div"); row.className = "pricing-rule";
row.innerHTML = `<div class="form-grid"><div class="field"><label>平台</label><select name="platform">${["openai", "anthropic", "gemini", "grok"].map(value => `<option value="${value}" ${rule.platform === value ? "selected" : ""}>${value.toUpperCase()}</option>`).join("")}</select></div><div class="field"><label>计费模式</label><select name="billing_mode"><option value="tokens" ${rule.billing_mode !== "request" ? "selected" : ""}>按 Token</option><option value="request" ${rule.billing_mode === "request" ? "selected" : ""}>按请求</option></select></div></div><div class="field"><label>模型</label><textarea name="models" class="compact-textarea" required>${escapeHtml((rule.models || []).join("\n"))}</textarea></div><div class="pricing-grid"><div class="field token-price"><label>输入 $ / 1M</label><input name="input_price" type="number" min="0" step="0.000001" value="${Number(rule.input_price || 0)}"></div><div class="field token-price"><label>缓存读取 $ / 1M</label><input name="cache_read_price" type="number" min="0" step="0.000001" value="${rule.cache_read_price == null ? "" : Number(rule.cache_read_price)}" placeholder="回退输入价"></div><div class="field token-price"><label>缓存写入 $ / 1M</label><input name="cache_write_price" type="number" min="0" step="0.000001" value="${rule.cache_write_price == null ? "" : Number(rule.cache_write_price)}"></div><div class="field token-price"><label>输出 $ / 1M</label><input name="output_price" type="number" min="0" step="0.000001" value="${Number(rule.output_price || 0)}"></div><div class="field request-price"><label>单次 $</label><input name="per_request_price" type="number" min="0" step="0.000001" value="${Number(rule.per_request_price || 0)}"></div><button class="button quiet small" type="button" data-remove-pricing>移除</button></div><div class="token-tier-editor"><div class="section-title compact"><h3>Token 区间</h3><button class="button quiet small" type="button" data-add-interval>添加区间</button></div><div class="pricing-intervals"></div></div>`;
const intervals = row.querySelector(".pricing-intervals");
(rule.intervals || []).forEach(interval => addPricingInterval(intervals, interval));
row.querySelector("[data-add-interval]").addEventListener("click", () => addPricingInterval(intervals));
row.querySelector("[data-remove-pricing]").addEventListener("click", () => row.remove());
const syncMode = () => { const tokens = row.querySelector('[name="billing_mode"]').value === "tokens"; row.querySelectorAll(".token-price, .token-tier-editor").forEach(element => { element.hidden = !tokens; }); row.querySelectorAll(".request-price").forEach(element => { element.hidden = tokens; }); };
row.querySelector('[name="billing_mode"]').addEventListener("change", syncMode);
syncMode(); container.append(row);
}
function addPricingInterval(container, interval = {}) {
const row = document.createElement("div"); row.className = "pricing-interval";
row.innerHTML = `<div class="field"><label>最小 Token（不含）</label><input name="interval_min" type="number" min="0" step="1" value="${Number(interval.min_tokens || 0)}" required></div><div class="field"><label>最大 Token（含）</label><input name="interval_max" type="number" min="1" step="1" value="${interval.max_tokens == null ? "" : Number(interval.max_tokens)}" placeholder="不设上限"></div><div class="field"><label>输入 $ / 1M</label><input name="interval_input" type="number" min="0" step="0.000001" value="${interval.input_price == null ? "" : Number(interval.input_price)}"></div><div class="field"><label>缓存读取 $ / 1M</label><input name="interval_cache_read" type="number" min="0" step="0.000001" value="${interval.cache_read_price == null ? "" : Number(interval.cache_read_price)}"></div><div class="field"><label>缓存写入 $ / 1M</label><input name="interval_cache_write" type="number" min="0" step="0.000001" value="${interval.cache_write_price == null ? "" : Number(interval.cache_write_price)}"></div><div class="field"><label>输出 $ / 1M</label><input name="interval_output" type="number" min="0" step="0.000001" value="${interval.output_price == null ? "" : Number(interval.output_price)}"></div><button class="button quiet small" type="button" data-remove-interval>移除</button>`;
row.querySelector("[data-remove-interval]").addEventListener("click", () => row.remove()); container.append(row);
}
async function saveChannel(id = null) {
const form = modal.querySelector("#channel-form"); if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
values.restrict_models = form.elements.restrict_models.checked;
values.group_ids = [...form.querySelectorAll('input[name="group_ids"]:checked')].map(input => Number(input.value));
const optionalNumber = input => input.value === "" ? null : Number(input.value);
values.model_pricing = [...form.querySelectorAll(".pricing-rule")].map(row => ({ platform: row.querySelector('[name="platform"]').value, billing_mode: row.querySelector('[name="billing_mode"]').value, models: parseModelList(row.querySelector('[name="models"]').value), input_price: Number(row.querySelector('[name="input_price"]').value), output_price: Number(row.querySelector('[name="output_price"]').value), cache_read_price: optionalNumber(row.querySelector('[name="cache_read_price"]')), cache_write_price: optionalNumber(row.querySelector('[name="cache_write_price"]')), per_request_price: Number(row.querySelector('[name="per_request_price"]').value), intervals: [...row.querySelectorAll(".pricing-interval")].map(interval => ({ min_tokens: Number(interval.querySelector('[name="interval_min"]').value), max_tokens: optionalNumber(interval.querySelector('[name="interval_max"]')), input_price: optionalNumber(interval.querySelector('[name="interval_input"]')), output_price: optionalNumber(interval.querySelector('[name="interval_output"]')), cache_read_price: optionalNumber(interval.querySelector('[name="interval_cache_read"]')), cache_write_price: optionalNumber(interval.querySelector('[name="interval_cache_write"]')) })) }));
const button = modal.querySelector("#save-channel"); button.disabled = true;
try { await api(id ? `/api/admin/channels/${id}` : "/api/admin/channels", { method: id ? "PUT" : "POST", body: JSON.stringify(values) }); closeModal(); toast(id ? "频道已更新" : "频道已创建"); await renderRoute(); }
catch (error) { modal.querySelector("#channel-error").textContent = error.message; button.disabled = false; }
}
async function handleChannelAction(event) {
const item = currentChannels.find(channel => String(channel.id) === String(event.currentTarget.dataset.id)); if (!item) return;
const action = event.currentTarget.dataset.channelAction;
if (action === "edit") return openChannelModal(item);
if (action === "delete") { openModal("删除频道", `<p>确认删除 <strong>${escapeHtml(item.name)}</strong>？</p><p class="form-error" id="channel-delete-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-channel-delete">删除</button>`); modal.querySelector("#confirm-channel-delete").addEventListener("click", async event => { event.currentTarget.disabled = true; try { await api(`/api/admin/channels/${item.id}`, { method: "DELETE" }); closeModal(); toast("频道已删除"); await renderRoute(); } catch (error) { modal.querySelector("#channel-delete-error").textContent = error.message; event.currentTarget.disabled = false; } }); return; }
try { await api(`/api/admin/channels/${item.id}`, { method: "PUT", body: JSON.stringify({ ...item, status: item.status === "active" ? "inactive" : "active" }) }); toast("频道状态已更新"); await renderRoute(); } catch (error) { toast(error.message, true); }
}
async function renderGroupAdmin(page) {
const [groups, accounts] = await Promise.all([
  api("/api/admin/groups"),
  api("/api/admin/accounts"),
]);
currentGroups = groups.data;
currentAccounts = accounts.data;
page.innerHTML = `${pageHeader("路由分组", `${currentGroups.length} 个分组`, `<button class="button" id="add-group">创建分组</button>`)}${currentGroups.length ? groupAdminTable(currentGroups) : emptyState("暂无路由分组", "创建分组并绑定上游账号", "创建分组", "empty-add-group")}`;
page.querySelector("#add-group")?.addEventListener("click", () => openGroupModal());
page.querySelector("#empty-add-group")?.addEventListener("click", () => openGroupModal());
page.querySelectorAll("[data-group-action]").forEach(button => button.addEventListener("click", handleGroupAction));
}
function groupAdminTable(groups) {
return `<div class="table-wrap"><table><thead><tr><th>分组</th><th>平台 / 类型</th><th>倍率</th><th>状态</th><th>账号</th><th>访问 / 订阅</th><th>模型策略</th><th></th></tr></thead><tbody>${groups.map(group => `<tr><td><span class="cell-main">${escapeHtml(group.name)}</span><span class="cell-sub">${escapeHtml(group.description || "")}</span></td><td><span class="cell-main">${escapeHtml(group.platform_label || group.platform.toUpperCase())}</span><span class="cell-sub">${group.subscription_type === "subscription" ? "订阅分组" : group.is_exclusive ? "专属标准分组" : "公共标准分组"}</span></td><td><span class="cell-main">${formatRateMultiplier(group.rate_multiplier)}</span><span class="cell-sub">${peakRateText(group) || "无高峰窗口"}</span></td><td>${group.enabled ? status("启用") : status("停用", "off")}</td><td>${group.account_ids.length}</td><td><span class="cell-main">${group.allowed_users || 0} 个授权</span><span class="cell-sub">${group.active_subscriptions || 0} 个活跃订阅</span></td><td>${group.allowed_models.length ? `${group.allowed_models.length} 个模型` : "全部模型"}</td><td><div class="cell-actions"><button class="button quiet small" data-group-action="rates" data-id="${group.id}">用户倍率</button><button class="button quiet small" data-group-action="edit" data-id="${group.id}">编辑</button><button class="button quiet small" data-group-action="delete" data-id="${group.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
}
function openGroupModal(group = null) {
const selectedAccounts = new Set(group?.account_ids || []);
openModal(group ? "编辑路由分组" : "创建路由分组", `<form id="group-form">
  <div class="field"><label for="group-name">名称</label><input id="group-name" name="name" value="${escapeHtml(group?.name || "")}" maxlength="80" required autofocus></div>
  <div class="field"><label for="group-description">说明</label><textarea id="group-description" name="description" class="compact-textarea">${escapeHtml(group?.description || "")}</textarea></div>
  <div class="form-grid"><div class="field"><label for="group-platform">平台标识</label><input id="group-platform" name="platform" value="${escapeHtml(group?.platform || "openai")}" maxlength="32" pattern="[A-Za-z0-9._-]+" required></div><div class="field"><label for="group-subscription-type">计费类型</label><select id="group-subscription-type" name="subscription_type"><option value="standard" ${group?.subscription_type !== "subscription" ? "selected" : ""}>标准</option><option value="subscription" ${group?.subscription_type === "subscription" ? "selected" : ""}>订阅</option></select></div></div>
  <label class="switch-row"><span><strong>专属分组</strong><small>标准分组启用后，仅显式授权用户可使用</small></span><input id="group-exclusive" type="checkbox" ${group?.is_exclusive ? "checked" : ""}></label>
  <div class="form-grid"><div class="field"><label for="group-rate">默认倍率</label><input id="group-rate" name="rate_multiplier" type="number" min="0.000001" max="1000" step="0.000001" value="${Number(group?.rate_multiplier ?? 1)}" required></div><div class="field"><label>高峰计费</label><label class="toggle-line"><input id="group-peak-enabled" type="checkbox" ${group?.peak_rate_enabled ? "checked" : ""}> 启用高峰倍率</label></div></div>
  <div class="form-grid" id="group-peak-fields"><div class="field"><label for="group-peak-start">开始时间</label><input id="group-peak-start" name="peak_start" type="time" value="${escapeHtml(group?.peak_start || "")}"></div><div class="field"><label for="group-peak-end">结束时间</label><input id="group-peak-end" name="peak_end" type="time" value="${escapeHtml(group?.peak_end || "")}"></div><div class="field"><label for="group-peak-rate">高峰倍率</label><input id="group-peak-rate" name="peak_rate_multiplier" type="number" min="0" max="1000" step="0.000001" value="${Number(group?.peak_rate_multiplier ?? 1)}"></div></div>
  <div class="form-grid"><div class="field"><label for="group-sort">排序</label><input id="group-sort" name="sort_order" type="number" min="-10000" max="10000" value="${Number(group?.sort_order || 0)}"></div><div class="field"><label>状态</label><label class="toggle-line"><input id="group-enabled" type="checkbox" ${group == null || group.enabled ? "checked" : ""}> 启用分组</label></div></div>
  <div class="field"><label>绑定上游账号</label><div class="choice-grid">${currentAccounts.length ? currentAccounts.map(account => `<label><input type="checkbox" name="account_ids" value="${account.id}" ${selectedAccounts.has(account.id) ? "checked" : ""}> <span>${escapeHtml(account.name)}</span><small>${account.kind === "oauth" ? "OAuth" : "API Key"}</small></label>`).join("") : `<span class="field-hint">暂无上游账号</span>`}</div></div>
  <div class="field"><label for="group-models">组级模型白名单</label><textarea id="group-models" name="allowed_models" class="compact-textarea" placeholder="每行一个模型 ID；留空允许全部">${escapeHtml((group?.allowed_models || []).join("\n"))}</textarea></div>
  <p class="form-error" id="group-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-group">保存</button>`);
const syncPeakFields = () => {
  const subscription = modal.querySelector("#group-subscription-type").value === "subscription";
  const enabled = subscription && modal.querySelector("#group-peak-enabled").checked;
  modal.querySelector("#group-peak-enabled").disabled = !subscription;
  modal.querySelector("#group-peak-fields").hidden = !enabled;
  modal.querySelectorAll("#group-peak-fields input").forEach(input => { input.disabled = !enabled; input.required = enabled; });
};
modal.querySelector("#group-subscription-type").addEventListener("change", syncPeakFields);
modal.querySelector("#group-peak-enabled").addEventListener("change", syncPeakFields);
syncPeakFields();
modal.querySelector("#save-group").addEventListener("click", () => saveGroup(group?.id));
}
async function saveGroup(id) {
const form = modal.querySelector("#group-form");
if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
values.enabled = form.querySelector("#group-enabled").checked;
values.is_exclusive = form.querySelector("#group-exclusive").checked;
values.peak_rate_enabled = form.querySelector("#group-subscription-type").value === "subscription" && form.querySelector("#group-peak-enabled").checked;
values.rate_multiplier = Number(values.rate_multiplier);
values.peak_rate_multiplier = Number(values.peak_rate_multiplier ?? 1);
values.peak_start = values.peak_start || "";
values.peak_end = values.peak_end || "";
values.sort_order = Number(values.sort_order);
values.allowed_models = parseModelList(values.allowed_models);
values.account_ids = [...form.querySelectorAll('input[name="account_ids"]:checked')].map(input => Number(input.value));
const button = modal.querySelector("#save-group"); button.disabled = true;
try {
  await api(id ? `/api/admin/groups/${id}` : "/api/admin/groups", { method: id ? "PUT" : "POST", body: JSON.stringify(values) });
  closeModal(); toast("路由分组已保存"); await renderRoute();
} catch (error) { modal.querySelector("#group-error").textContent = error.message; }
finally { button.disabled = false; }
}
async function handleGroupAction(event) {
const { groupAction: action, id } = event.currentTarget.dataset;
const group = currentGroups.find(item => String(item.id) === id);
if (action === "edit" && group) return openGroupModal(group);
if (action === "rates" && group) return openGroupRateModal(group);
if (action === "delete" && !confirm("删除后关联 Key 会恢复为未分组，确认继续？")) return;
try { await api(`/api/admin/groups/${id}`, { method: "DELETE" }); toast("路由分组已删除"); await renderRoute(); }
catch (error) { toast(error.message, true); }
}
async function openGroupRateModal(group) {
try {
  const [ratesResult, usersResult] = await Promise.all([
    api(`/api/admin/groups/${group.id}/rate-multipliers`),
    api("/api/admin/users"),
  ]);
  const entries = ratesResult.data.map(entry => ({ ...entry }));
  const users = usersResult.data;
  openModal(`用户专属倍率 · ${group.name}`, `<div class="group-rate-editor"><div class="form-grid"><div class="field"><label for="rate-user">用户</label><select id="rate-user"></select></div><div class="field"><label for="rate-value">倍率</label><input id="rate-value" type="number" min="0.000001" max="1000" step="0.000001" value="1"></div></div><button class="button secondary" id="add-user-rate">添加或更新</button><div id="group-rate-rows"></div><p class="form-error" id="group-rate-error"></p></div>`, `<button class="button secondary" id="clear-group-rates">全部清除</button><button class="button" id="save-group-rates">保存</button>`);
  const redraw = () => {
    const selected = modal.querySelector("#rate-user").value;
    modal.querySelector("#rate-user").innerHTML = users.map(user => `<option value="${user.id}" ${String(user.id) === selected ? "selected" : ""}>${escapeHtml(user.display_name || user.username)} · ${escapeHtml(user.email || user.username)}</option>`).join("");
    modal.querySelector("#group-rate-rows").innerHTML = entries.length ? `<div class="table-wrap"><table><thead><tr><th>用户</th><th>专属倍率</th><th></th></tr></thead><tbody>${entries.map(entry => `<tr><td><span class="cell-main">${escapeHtml(entry.display_name || entry.user_name)}</span><span class="cell-sub">${escapeHtml(entry.user_email || entry.user_name)}</span></td><td>${formatRateMultiplier(entry.rate_multiplier)}</td><td><button class="button quiet small" data-remove-rate="${entry.user_id}">移除</button></td></tr>`).join("")}</tbody></table></div>` : `<p class="cell-sub rate-empty">尚未设置用户专属倍率</p>`;
    modal.querySelectorAll("[data-remove-rate]").forEach(button => button.addEventListener("click", () => { const index = entries.findIndex(entry => String(entry.user_id) === button.dataset.removeRate); if (index >= 0) entries.splice(index, 1); redraw(); }));
  };
  modal.querySelector("#add-user-rate").addEventListener("click", () => {
    const userId = Number(modal.querySelector("#rate-user").value);
    const rate = Number(modal.querySelector("#rate-value").value);
    const user = users.find(item => item.id === userId);
    if (!user || !Number.isFinite(rate) || rate <= 0 || rate > 1000) { modal.querySelector("#group-rate-error").textContent = "请选择用户并输入大于 0 且不超过 1000 的倍率"; return; }
    const entry = { user_id: user.id, user_name: user.username, display_name: user.display_name, user_email: user.email, rate_multiplier: rate };
    const index = entries.findIndex(item => item.user_id === user.id);
    if (index >= 0) entries[index] = entry; else entries.push(entry);
    entries.sort((a, b) => String(a.user_name).localeCompare(String(b.user_name)));
    modal.querySelector("#group-rate-error").textContent = ""; redraw();
  });
  modal.querySelector("#clear-group-rates").addEventListener("click", () => { entries.splice(0); redraw(); });
  modal.querySelector("#save-group-rates").addEventListener("click", async event => {
    event.currentTarget.disabled = true;
    try { await api(`/api/admin/groups/${group.id}/rate-multipliers`, { method: "PUT", body: JSON.stringify({ entries: entries.map(entry => ({ user_id: entry.user_id, rate_multiplier: entry.rate_multiplier })) }) }); closeModal(); toast("用户专属倍率已保存"); await renderRoute(); }
    catch (error) { modal.querySelector("#group-rate-error").textContent = error.message; event.currentTarget.disabled = false; }
  });
  redraw();
} catch (error) { toast(error.message, true); }
}
function orderTable(items, admin) {
return `<div class="table-wrap"><table class="order-table"><thead><tr>${admin ? "<th>用户</th>" : ""}<th>订单</th><th>套餐</th><th>金额</th><th>状态</th><th>时间</th>${admin ? "<th></th>" : ""}</tr></thead><tbody>${items.map(item => `<tr>${admin ? `<td><span class="cell-main mono">${escapeHtml(item.username)}</span><span class="cell-sub">${escapeHtml(item.email || "-")}</span></td>` : ""}<td class="mono">#${item.id}</td><td>${escapeHtml(item.plan_name)}</td><td>${formatMoney(item.amount_cents)}</td><td>${item.status === "paid" ? status("已支付") : item.status === "refunded" ? status("已退款", "off") : status(item.status, "warn")}</td><td>${formatDate(item.created_at)}</td>${admin ? `<td>${item.status === "paid" ? `<button class="button quiet small" data-refund-order="${item.id}" data-amount="${item.amount_cents}">退款</button>` : ""}</td>` : ""}</tr>`).join("")}</tbody></table></div>`;
}
async function renderOrderAdmin(page) {
const [dashboard, orders] = await Promise.all([
  api("/api/admin/orders/dashboard"),
  api("/api/admin/orders"),
]);
page.innerHTML = `
  ${pageHeader("订单管理", `${dashboard.data.orders} 个订单`)}
  <section class="metric-grid">${metric("已支付订单", dashboard.data.paid_orders)}${metric("当前收入", formatMoney(dashboard.data.revenue_cents), "good")}${metric("累计退款", formatMoney(dashboard.data.refunded_cents))}</section>
  ${orders.data.length ? orderTable(orders.data, true) : emptyState("暂无订单", "用户使用余额购买套餐后会显示")}`;
page.querySelectorAll("[data-refund-order]").forEach(button => button.addEventListener("click", refundOrder));
}
function refundOrder(event) {
const { refundOrder: orderId, amount } = event.currentTarget.dataset;
openModal("确认退款", `<p>退回 <strong>${formatMoney(amount)}</strong> 到用户账户余额。</p><p class="field-hint">关联订阅会立即取消。</p><p class="form-error" id="refund-error"></p>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-refund">确认退款</button>');
modal.querySelector("#confirm-refund").addEventListener("click", () => executeRefund(orderId));
}
async function executeRefund(orderId) {
const button = modal.querySelector("#confirm-refund");
button.disabled = true;
try {
  await api(`/api/admin/orders/${orderId}/refund`, { method: "POST", body: "{}" });
  closeModal(); toast("订单已退款"); await renderRoute();
} catch (error) { modal.querySelector("#refund-error").textContent = error.message; button.disabled = false; }
}
async function renderRedeem(page) {
const history = await api("/api/user/redemptions");
page.innerHTML = `
  ${pageHeader("兑换码", "兑换后立即获得对应套餐")}
  <section class="redeem-panel"><form id="redeem-form"><div class="field"><label for="redeem-code">兑换码</label><input id="redeem-code" name="code" class="mono" autocomplete="off" placeholder="mini-redeem_..." required></div><button class="button" type="submit">立即兑换</button><p class="form-error" id="redeem-error"></p></form></section>
  <section class="section"><div class="section-title"><h2>兑换历史</h2></div>${history.data.length ? `<div class="table-wrap"><table><thead><tr><th>名称</th><th>前缀</th><th>兑换时间</th><th>订阅</th></tr></thead><tbody>${history.data.map(item => `<tr><td class="cell-main">${escapeHtml(item.name)}</td><td class="mono">${escapeHtml(item.code_prefix)}...</td><td>${formatDate(item.redeemed_at)}</td><td>#${item.subscription_id}</td></tr>`).join("")}</tbody></table></div>` : emptyState("暂无兑换记录", "")}</section>`;
page.querySelector("#redeem-form").addEventListener("submit", submitRedeemCode);
}
async function submitRedeemCode(event) {
event.preventDefault(); const form = event.currentTarget; const button = form.querySelector("button"); const error = form.querySelector("#redeem-error");
button.disabled = true; error.textContent = "";
try {
  const result = await api("/api/user/redeem", { method: "POST", body: JSON.stringify(Object.fromEntries(new FormData(form))) });
  form.reset(); toast(`已兑换 ${result.data.plan_name}`); await renderRoute();
} catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
async function renderRedeemAdmin(page) {
const [codes, plans] = await Promise.all([
  api("/api/admin/redeem-codes"),
  api("/api/admin/plans"),
]);
currentRedeemCodes = codes.data;
currentPlans = plans.data;
page.innerHTML = `${pageHeader("兑换码管理", `${codes.data.length} 个兑换码`, `<button class="button" id="add-redeem-code" ${plans.data.some(plan => plan.enabled) ? "" : "disabled"}>生成兑换码</button>`)}${codes.data.length ? redeemCodeTable(codes.data) : emptyState("暂无兑换码", plans.data.length ? "生成后完整码仅显示一次" : "请先创建并启用套餐")}`;
page.querySelector("#add-redeem-code")?.addEventListener("click", openRedeemCodeModal);
page.querySelectorAll("[data-redeem-action]").forEach(button => button.addEventListener("click", handleRedeemAdminAction));
}
function redeemCodeTable(codes) {
return `<div class="table-wrap"><table class="redeem-code-table"><thead><tr><th>名称</th><th>前缀</th><th>套餐</th><th>使用</th><th>到期</th><th>状态</th><th></th></tr></thead><tbody>${codes.map(code => `<tr><td class="cell-main">${escapeHtml(code.name)}</td><td class="mono">${escapeHtml(code.code_prefix)}...</td><td>${escapeHtml(code.plan_name)}</td><td>${code.used_count} / ${code.max_uses}</td><td>${formatDate(code.expires_at)}</td><td>${code.enabled && code.used_count < code.max_uses ? status("可用") : status("停用", "off")}</td><td><div class="cell-actions"><button class="button quiet small" data-redeem-action="toggle" data-id="${code.id}" data-enabled="${code.enabled}">${code.enabled ? "停用" : "启用"}</button>${code.used_count === 0 ? `<button class="button quiet small" data-redeem-action="delete" data-id="${code.id}">删除</button>` : ""}</div></td></tr>`).join("")}</tbody></table></div>`;
}
function openRedeemCodeModal() {
openModal("生成兑换码", `<form id="redeem-code-form"><div class="field"><label for="redeem-name">名称</label><input id="redeem-name" name="name" maxlength="80" placeholder="活动赠送" required autofocus></div><div class="field"><label for="redeem-plan">套餐</label><select id="redeem-plan" name="plan_id">${currentPlans.filter(plan => plan.enabled).map(plan => `<option value="${plan.id}">${escapeHtml(plan.name)}</option>`).join("")}</select></div><div class="form-grid"><div class="field"><label for="redeem-token-limit">覆盖 Token 上限</label><input id="redeem-token-limit" name="token_limit" type="number" min="0" placeholder="留空使用套餐值"></div><div class="field"><label for="redeem-days">覆盖有效天数</label><input id="redeem-days" name="duration_days" type="number" min="1" max="3650" placeholder="留空使用套餐值"></div></div><div class="form-grid"><div class="field"><label for="redeem-max-uses">可用次数</label><input id="redeem-max-uses" name="max_uses" type="number" min="1" max="100000" value="1" required></div><div class="field"><label for="redeem-expiry">兑换有效天数</label><input id="redeem-expiry" name="expires_in_days" type="number" min="1" max="3650" placeholder="留空永不过期"></div></div><p class="form-error" id="redeem-code-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-redeem-code">生成</button>`);
modal.querySelector("#save-redeem-code").addEventListener("click", saveRedeemCode);
}
async function saveRedeemCode() {
const form = modal.querySelector("#redeem-code-form"); if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
values.plan_id = Number(values.plan_id); values.max_uses = Number(values.max_uses);
for (const key of ["token_limit", "duration_days", "expires_in_days"]) values[key] = values[key] === "" ? null : Number(values[key]);
const button = modal.querySelector("#save-redeem-code"); button.disabled = true;
try {
  const result = await api("/api/admin/redeem-codes", { method: "POST", body: JSON.stringify(values) });
  openModal("兑换码已生成", `<p>完整兑换码仅显示一次。</p><div class="secret-box mono">${escapeHtml(result.data.code)}</div>`, `<button class="button secondary" id="copy-redeem-code">复制</button><button class="button" id="finish-redeem-code">完成</button>`);
  modal.querySelector("#copy-redeem-code").addEventListener("click", async () => { await navigator.clipboard.writeText(result.data.code); toast("已复制"); });
  modal.querySelector("#finish-redeem-code").addEventListener("click", async () => { closeModal(); await renderRoute(); });
} catch (error) { modal.querySelector("#redeem-code-error").textContent = error.message; }
finally { button.disabled = false; }
}
async function handleRedeemAdminAction(event) {
const { redeemAction: action, id, enabled } = event.currentTarget.dataset;
if (action === "delete" && !confirm("确认删除这个未使用兑换码？")) return;
try {
  await api(`/api/admin/redeem-codes/${id}`, action === "toggle" ? { method: "PUT", body: JSON.stringify({ enabled: enabled !== "true" }) } : { method: "DELETE" });
  toast(action === "delete" ? "兑换码已删除" : "兑换码状态已更新"); await renderRoute();
} catch (error) { toast(error.message, true); }
}
async function renderAnnouncements(page) {
await loadFeatureScript("engagement");
return window.Sub2MiniEngagement.renderAnnouncements(page);
}
function announcementCards(items, compact = false) {
return `<div class="announcement-list ${compact ? "compact" : ""}">${items.map(item => `<article class="announcement-item ${item.is_read ? "read" : ""}">
  <header><div><span class="cell-sub">${formatDate(item.created_at)}</span><h2>${escapeHtml(item.title)}</h2></div>${item.notify_mode === "popup" ? status("重要", "warn") : ""}</header>
  <div class="content-body markdown-body">${item.rendered_html || escapeHtml(item.content)}</div>
  ${item.is_read === undefined ? "" : item.is_read ? `<span class="cell-sub">已读</span>` : `<button class="button quiet small" data-read-announcement="${item.id}">标记已读</button>`}
</article>`).join("")}</div>`;
}
async function markAnnouncementRead(event) {
await loadFeatureScript("engagement");
return window.Sub2MiniEngagement.markAnnouncementRead(event);
}
async function renderPages(page) {
await loadFeatureScript("content");
return window.Sub2MiniContent.renderPages(page);
}
async function renderContentAdmin(page) {
await loadFeatureScript("engagement");
return window.Sub2MiniEngagement.renderContentAdmin(page);
}
async function renderUsage(page) {
await loadFeatureScript("usage");
return window.Sub2MiniUsage.render(page);
}
async function renderProfile(page) {
const [result, totpResult] = await Promise.all([api("/api/user/profile"), api("/api/user/totp/status")]);
const profile = result.data;
const totp = totpResult.data;
page.innerHTML = `
  ${pageHeader("个人资料", "账户信息与登录安全")}
  <section class="profile-summary">
    <div><span>用户名</span><strong class="mono">${escapeHtml(profile.username)}</strong></div>
    <div><span>邮箱</span><strong>${escapeHtml(profile.email || "未设置")}</strong></div>
    <div><span>角色</span><strong>${profile.role === "admin" ? "管理员" : "普通用户"}</strong></div>
    <div><span>API Key</span><strong>${formatNumber(profile.key_count)}</strong></div>
    <div><span>累计请求</span><strong>${formatNumber(profile.total_requests)}</strong></div>
    <div><span>累计 Token</span><strong>${formatNumber(profile.total_tokens)}</strong></div>
    <div><span>账户余额</span><strong>${formatMoney(profile.balance_cents)}</strong></div>
    <div><span>邮箱状态</span><strong>${profile.email ? profile.email_verified ? "已验证" : "未验证" : "-"}</strong></div>
  </section>
  <div class="settings-grid">
    <section class="settings-panel">
      <div class="settings-heading"><h2>基本信息</h2><p>显示名称会出现在控制台中。</p></div>
      <form id="profile-form">
        <div class="field"><label for="profile-username">用户名</label><input id="profile-username" value="${escapeHtml(profile.username)}" disabled></div>
        <div class="field"><label for="profile-display-name">显示名称</label><input id="profile-display-name" name="display_name" value="${escapeHtml(profile.display_name)}" maxlength="80" required></div>
        <button class="button" type="submit">保存资料</button>
        <p class="form-error" id="profile-error"></p>
      </form>
    </section>
    <section class="settings-panel">
      <div class="settings-heading"><h2>登录邮箱</h2><p>${profile.email ? `当前邮箱已${profile.email_verified ? "验证" : "设置"}` : "尚未绑定邮箱"}</p></div>
      <div class="field"><label for="profile-email">邮箱</label><input id="profile-email" value="${escapeHtml(profile.email || "未设置")}" disabled></div>
      ${profile.pending_email ? `<p class="auth-notice">待验证：${escapeHtml(profile.pending_email)}，有效期至 ${formatDate(profile.pending_email_expires_at)}</p>` : ""}
      <div class="actions">
        <button class="button" id="change-profile-email" ${state.mailConfigured ? "" : "disabled"}>${profile.email ? "更换邮箱" : "绑定邮箱"}</button>
        ${profile.pending_email ? '<a class="button secondary" href="#/email-verify">继续验证</a>' : ""}
        ${profile.email ? '<button class="button danger" id="remove-profile-email">移除邮箱</button>' : ""}
      </div>
      ${state.mailConfigured ? "" : '<p class="field-hint">管理员尚未配置邮件投递，暂时不能绑定或更换邮箱。</p>'}
    </section>
    <section class="settings-panel">
      <div class="settings-heading"><h2>修改密码</h2><p>修改后其他设备上的会话会立即失效。</p></div>
      <form id="change-password-form">
        <div class="field"><label for="current-password">当前密码</label><input id="current-password" name="current_password" type="password" autocomplete="current-password" required></div>
        <div class="field"><label for="new-profile-password">新密码</label><input id="new-profile-password" name="new_password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required></div>
        <div class="field"><label for="confirm-profile-password">确认新密码</label><input id="confirm-profile-password" name="confirm_password" type="password" minlength="8" maxlength="128" autocomplete="new-password" required></div>
        <button class="button" type="submit">更新密码</button>
        <p class="form-error" id="password-change-error"></p>
      </form>
    </section>
    <section class="settings-panel">
      <div class="settings-heading"><h2>双因素认证</h2><p>${totp.enabled ? `已启用 · ${formatDate(totp.enabled_at)}` : "当前未启用"}</p></div>
      ${totp.enabled ? `<button class="button danger" id="disable-totp">停用 TOTP</button>` : `<button class="button" id="enable-totp">启用 TOTP</button>`}
    </section>
  </div>`;
page.querySelector("#profile-form").addEventListener("submit", saveProfile);
page.querySelector("#change-password-form").addEventListener("submit", saveOwnPassword);
page.querySelector("#change-profile-email")?.addEventListener("click", () => openEmailChange(profile));
page.querySelector("#remove-profile-email")?.addEventListener("click", () => openEmailRemoval(profile));
page.querySelector("#enable-totp")?.addEventListener("click", openTotpSetup);
page.querySelector("#disable-totp")?.addEventListener("click", openTotpDisable);
}
function openEmailChange(profile) {
openModal(profile.email ? "更换登录邮箱" : "绑定登录邮箱", `<form id="email-change-form">
  <div class="field"><label for="new-profile-email">新邮箱</label><input id="new-profile-email" name="email" type="email" maxlength="254" autocomplete="email" required autofocus></div>
  <div class="field"><label for="email-current-password">当前密码</label><input id="email-current-password" name="current_password" type="password" autocomplete="current-password" required></div>
  <p class="field-hint">验证码会发送到新邮箱，10 分钟内有效。</p><p class="form-error" id="email-change-error"></p>
</form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button" id="request-email-change">发送验证码</button>');
modal.querySelector("#request-email-change").addEventListener("click", async event => {
  const form = modal.querySelector("#email-change-form");
  if (!form.reportValidity()) return;
  event.currentTarget.disabled = true;
  try {
    const result = await api("/api/user/profile/email/request", { method: "POST", body: JSON.stringify(Object.fromEntries(new FormData(form))) });
    closeModal();
    location.hash = `#/email-verify?email=${encodeURIComponent(result.data.email)}`;
    toast("验证码已发送");
  } catch (error) {
    modal.querySelector("#email-change-error").textContent = error.message;
    event.currentTarget.disabled = false;
  }
});
}
function openEmailRemoval(profile) {
openModal("移除登录邮箱", `<form id="email-remove-form"><p>移除 <strong>${escapeHtml(profile.email)}</strong> 后将不能使用邮箱登录或找回密码。</p><div class="field"><label for="remove-email-password">当前密码</label><input id="remove-email-password" name="current_password" type="password" autocomplete="current-password" required autofocus></div><p class="form-error" id="email-remove-error"></p></form>`, '<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-email-removal">移除邮箱</button>');
modal.querySelector("#confirm-email-removal").addEventListener("click", async event => {
  const form = modal.querySelector("#email-remove-form");
  if (!form.reportValidity()) return;
  event.currentTarget.disabled = true;
  try {
    await api("/api/user/profile/email", { method: "DELETE", body: JSON.stringify(Object.fromEntries(new FormData(form))) });
    closeModal(); toast("邮箱已移除，其他会话已退出"); await renderRoute();
  } catch (error) {
    modal.querySelector("#email-remove-error").textContent = error.message;
    event.currentTarget.disabled = false;
  }
});
}
async function renderEmailVerification(page) {
const result = await api("/api/user/profile");
const profile = result.data;
const params = new URLSearchParams(location.hash.split("?", 2)[1] || "");
const email = params.get("email") || profile.pending_email || "";
page.innerHTML = `${pageHeader("验证邮箱", "输入发送到新邮箱的一次性验证码", '<a class="button secondary" href="#/profile">返回个人资料</a>')}
  ${email ? `<section class="settings-panel email-verification-panel"><div class="settings-heading"><h2>${escapeHtml(email)}</h2><p>验证码在发送后 10 分钟内有效，验证成功后其他设备会话将退出。</p></div><form id="email-confirm-form"><input name="email" type="hidden" value="${escapeHtml(email)}"><div class="field"><label for="profile-email-code">邮箱验证码</label><input id="profile-email-code" name="code" maxlength="16" autocomplete="one-time-code" required autofocus></div><button class="button" type="submit">确认邮箱</button><p class="form-error" id="email-confirm-error"></p></form></section>` : emptyState("没有待验证邮箱", "请先从个人资料页发起绑定或更换邮箱", "返回个人资料", "back-to-profile")}`;
page.querySelector("#back-to-profile")?.addEventListener("click", () => { location.hash = "#/profile"; });
page.querySelector("#email-confirm-form")?.addEventListener("submit", async event => {
  event.preventDefault();
  const form = event.currentTarget;
  const button = form.querySelector("button");
  const error = form.querySelector("#email-confirm-error");
  button.disabled = true; error.textContent = "";
  try {
    await api("/api/user/profile/email/confirm", { method: "POST", body: JSON.stringify(Object.fromEntries(new FormData(form))) });
    toast("邮箱已验证，其他会话已退出"); location.hash = "#/profile";
  } catch (requestError) {
    error.textContent = requestError.message;
    button.disabled = false;
  }
});
}
function openTotpSetup() {
openModal("启用双因素认证", `<form id="totp-password-form"><div class="field"><label for="totp-current-password">当前密码</label><input id="totp-current-password" name="password" type="password" autocomplete="current-password" required autofocus></div><p class="form-error" id="totp-setup-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="start-totp-setup">继续</button>`);
modal.querySelector("#start-totp-setup").addEventListener("click", async event => {
  const form = modal.querySelector("#totp-password-form"); if (!form.reportValidity()) return; event.currentTarget.disabled = true;
  try { const result = await api("/api/user/totp/setup", { method: "POST", body: JSON.stringify({ password: form.elements.password.value }) }); openTotpVerification(result.data); }
  catch (error) { modal.querySelector("#totp-setup-error").textContent = error.message; event.currentTarget.disabled = false; }
});
}
function openTotpVerification(data) {
openModal("验证身份验证器", `<div class="secret-box mono">${escapeHtml(data.secret)}</div><div class="field"><label for="totp-uri">配置 URI</label><input id="totp-uri" value="${escapeHtml(data.otpauth_uri)}" readonly></div><form id="totp-enable-form"><div class="field"><label for="totp-enable-code">6 位动态码</label><input id="totp-enable-code" name="totp_code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code" required autofocus></div><p class="form-error" id="totp-enable-error"></p></form>`, `<button class="button secondary" id="copy-totp-uri">复制 URI</button><button class="button" id="confirm-totp-enable">启用</button>`);
modal.querySelector("#copy-totp-uri").addEventListener("click", async () => { await navigator.clipboard.writeText(data.otpauth_uri); toast("URI 已复制"); });
modal.querySelector("#confirm-totp-enable").addEventListener("click", async event => {
  const form = modal.querySelector("#totp-enable-form"); if (!form.reportValidity()) return; event.currentTarget.disabled = true;
  try { const result = await api("/api/user/totp/enable", { method: "POST", body: JSON.stringify({ totp_code: form.elements.totp_code.value }) }); openRecoveryCodes(result.data.recovery_codes); }
  catch (error) { modal.querySelector("#totp-enable-error").textContent = error.message; event.currentTarget.disabled = false; }
});
}
function openRecoveryCodes(codes) {
openModal("恢复码", `<div class="recovery-code-grid">${codes.map(code => `<code>${escapeHtml(code)}</code>`).join("")}</div>`, `<button class="button secondary" id="copy-recovery-codes">复制全部</button><button class="button" id="finish-totp">完成</button>`);
modal.querySelector("#copy-recovery-codes").addEventListener("click", async () => { await navigator.clipboard.writeText(codes.join("\n")); toast("恢复码已复制"); });
modal.querySelector("#finish-totp").addEventListener("click", async () => { closeModal(); await renderRoute(); });
}
function openTotpDisable() {
openModal("停用双因素认证", `<form id="totp-disable-form"><div class="field"><label for="disable-totp-password">当前密码</label><input id="disable-totp-password" name="password" type="password" autocomplete="current-password" required autofocus></div><div class="field"><label for="disable-totp-code">6 位动态码</label><input id="disable-totp-code" name="totp_code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code" required></div><p class="form-error" id="totp-disable-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-disable-totp">停用</button>`);
modal.querySelector("#confirm-disable-totp").addEventListener("click", async event => {
  const form = modal.querySelector("#totp-disable-form"); if (!form.reportValidity()) return; event.currentTarget.disabled = true;
  try { await api("/api/user/totp/disable", { method: "POST", body: JSON.stringify(Object.fromEntries(new FormData(form))) }); closeModal(); toast("双因素认证已停用"); await renderRoute(); }
  catch (error) { modal.querySelector("#totp-disable-error").textContent = error.message; event.currentTarget.disabled = false; }
});
}
async function saveProfile(event) {
event.preventDefault();
const form = event.currentTarget;
const button = form.querySelector("button");
const error = form.querySelector("#profile-error");
button.disabled = true;
error.textContent = "";
try {
  const result = await api("/api/user/profile", {
    method: "PUT",
    body: JSON.stringify({ display_name: form.elements.display_name.value }),
  });
  state.displayName = result.data.display_name;
  const accountName = document.querySelector(".account-copy strong");
  const dropdownName = document.querySelector(".account-dropdown-head strong");
  if (accountName) accountName.textContent = state.displayName;
  if (dropdownName) dropdownName.textContent = state.displayName;
  toast("个人资料已更新");
} catch (requestError) {
  error.textContent = requestError.message;
} finally {
  button.disabled = false;
}
}
async function saveOwnPassword(event) {
event.preventDefault();
const form = event.currentTarget;
const button = form.querySelector("button");
const error = form.querySelector("#password-change-error");
const values = Object.fromEntries(new FormData(form));
error.textContent = "";
if (values.new_password !== values.confirm_password) {
  error.textContent = "两次输入的新密码不一致";
  return;
}
button.disabled = true;
try {
  await api("/api/user/password", {
    method: "PUT",
    body: JSON.stringify({ current_password: values.current_password, new_password: values.new_password }),
  });
  form.reset();
  toast("密码已更新，其他会话已退出");
} catch (requestError) {
  error.textContent = requestError.message;
} finally {
  button.disabled = false;
}
}
async function renderPublicHome() {
await loadFeatureScript("content");
return window.Sub2MiniContent.renderHome();
}
async function renderPublicPage(slug) {
await loadFeatureScript("content");
return window.Sub2MiniContent.renderPage(slug);
}
function renderPublicKeyUsage() {
app.innerHTML = `
  <main class="public-screen">
    <header class="public-topbar">
      <a class="public-brand" href="#/overview"><img src="${siteLogo()}" alt=""><span>${escapeHtml(state.siteName)}</span></a>
      <a class="button secondary" href="#/overview">${state.user ? "返回控制台" : "账户登录"}</a>
    </header>
    <div class="public-content">
      ${pageHeader("API Key 用量", "按密钥查询请求与 Token 使用情况")}
      <section class="query-panel">
        <form id="key-usage-form">
          <div class="query-grid">
            <div class="field query-key-field"><label for="usage-api-key">API Key</label><input id="usage-api-key" name="api_key" type="password" autocomplete="off" placeholder="sk-mini_..." required></div>
            <div class="field"><label for="usage-range">时间范围</label><select id="usage-range" name="range"><option value="today">今天</option><option value="7d" selected>最近 7 天</option><option value="30d">最近 30 天</option><option value="all">全部</option><option value="custom">自定义</option></select></div>
            <button class="button query-button" type="submit">查询</button>
          </div>
          <div class="custom-dates" id="custom-dates" hidden>
            <div class="field"><label for="usage-start">开始日期</label><input id="usage-start" name="start_date" type="date"></div>
            <div class="field"><label for="usage-end">结束日期</label><input id="usage-end" name="end_date" type="date"></div>
          </div>
          <p class="form-error" id="key-usage-error"></p>
        </form>
      </section>
      <div id="key-usage-result"></div>
    </div>
  </main>`;
const form = document.querySelector("#key-usage-form");
form.elements.range.addEventListener("change", () => {
  const custom = form.elements.range.value === "custom";
  document.querySelector("#custom-dates").hidden = !custom;
  form.elements.start_date.required = custom;
  form.elements.end_date.required = custom;
});
form.addEventListener("submit", queryKeyUsage);
}
async function queryKeyUsage(event) {
event.preventDefault();
const form = event.currentTarget;
if (!form.reportValidity()) return;
const button = form.querySelector("button[type=submit]");
const error = form.querySelector("#key-usage-error");
const resultArea = document.querySelector("#key-usage-result");
const values = Object.fromEntries(new FormData(form));
button.disabled = true;
error.textContent = "";
resultArea.innerHTML = `<div class="boot-screen compact"><p>正在查询</p></div>`;
try {
  const result = await api("/api/public/key-usage", { method: "POST", body: JSON.stringify(values) });
  resultArea.innerHTML = keyUsageResult(result.data);
} catch (requestError) {
  resultArea.innerHTML = "";
  error.textContent = requestError.message;
} finally {
  button.disabled = false;
}
}
function keyUsageResult(data) {
const stats = data.stats;
return `
  <section class="key-result-header">
    <div><span class="cell-sub">密钥</span><strong>${escapeHtml(data.key.name)}</strong><span class="mono cell-sub">${escapeHtml(data.key.token_prefix)}...</span></div>
    ${data.key.enabled ? status("有效") : status("已停用", "off")}
  </section>
  <section class="metric-grid key-metrics">
    ${metric("请求", formatNumber(stats.requests), "good")}
    ${metric("成功", formatNumber(stats.successful_requests), "good")}
    ${metric("失败", formatNumber(stats.failed_requests), stats.failed_requests ? "warn" : "good")}
    ${metric("输入 Token", formatNumber(stats.input_tokens))}
    ${metric("输出 Token", formatNumber(stats.output_tokens))}
    ${metric("缓存 Token", formatNumber(stats.cached_input_tokens))}
    ${metric("推理 Token", formatNumber(stats.reasoning_tokens))}
    ${metric("总 Token", formatNumber(stats.total_tokens))}
    ${metric("成本", formatUsdMicros(stats.cost_microusd))}
    ${metric("平均耗时", `${formatNumber(stats.average_duration_ms)} ms`)}
  </section>
  <section class="key-policy-summary">
    <div><span>状态</span><strong>${escapeHtml(data.key.status || (data.key.enabled ? "active" : "inactive"))}</strong></div>
    <div><span>总 Token 额度</span><strong>${data.key.quota_tokens ? `${formatNumber(data.key.used_tokens)} / ${formatNumber(data.key.quota_tokens)}` : "无限"}</strong></div>
    <div><span>总消费额度</span><strong>${data.key.quota_cost_microusd ? `${formatUsdMicros(data.key.used_cost_microusd)} / ${formatUsdMicros(data.key.quota_cost_microusd)}` : "无限"}</strong></div>
    <div><span>5h / 1d / 7d</span><strong>${formatUsdMicros(data.key.usage_5h_microusd)} / ${formatUsdMicros(data.key.usage_1d_microusd)} / ${formatUsdMicros(data.key.usage_7d_microusd)}</strong></div>
    <div><span>模型策略</span><strong>${data.key.allowed_model_count ? `${data.key.allowed_model_count} 个模型` : "全部模型"}</strong></div>
    <div><span>IP 策略</span><strong>白名单 ${data.key.ip_whitelist_count} · 黑名单 ${data.key.ip_blacklist_count}</strong></div>
  </section>
  <div class="usage-breakdown">
    <section>
      <div class="section-title"><h2>模型分布</h2></div>
      ${data.models.length ? `<div class="table-wrap"><table><thead><tr><th>模型</th><th>请求</th><th>Token</th></tr></thead><tbody>${data.models.map(row => `<tr><td>${escapeHtml(row.model)}</td><td>${formatNumber(row.requests)}</td><td>${formatNumber(row.tokens)}</td></tr>`).join("")}</tbody></table></div>` : emptyState("暂无模型数据", "所选时间范围内没有请求")}
    </section>
    <section>
      <div class="section-title"><h2>每日用量</h2></div>
      ${data.trend.length ? `<div class="table-wrap"><table><thead><tr><th>日期</th><th>请求</th><th>Token</th></tr></thead><tbody>${data.trend.map(row => `<tr><td>${escapeHtml(row.date)}</td><td>${formatNumber(row.requests)}</td><td>${formatNumber(row.tokens)}</td></tr>`).join("")}</tbody></table></div>` : emptyState("暂无每日数据", "所选时间范围内没有请求")}
    </section>
  </div>`;
}
function usageTable(rows, full = false) {
if (!rows.length) return emptyState("暂无请求记录", "网关请求完成后将在这里出现");
const userHeader = state.role === "admin" ? "<th>用户</th>" : "";
return `<div class="table-wrap"><table><thead><tr><th>时间</th>${userHeader}<th>端点</th><th>模型</th><th>状态</th><th>Token</th><th>成本</th>${full ? "<th class=\"hide-mobile\">类型</th><th class=\"hide-mobile\">耗时</th><th class=\"hide-mobile\">请求 ID</th><th></th>" : ""}</tr></thead>
  <tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td>${state.role === "admin" ? `<td class="mono">${row.user_id ? `#${row.user_id}` : "system"}</td>` : ""}<td class="mono">${escapeHtml(row.endpoint)}</td><td>${escapeHtml(row.model || "-")}${row.mapped_model ? `<span class="cell-sub">→ ${escapeHtml(row.mapped_model)}</span>` : ""}</td><td>${row.status_code < 400 ? status(String(row.status_code)) : status(String(row.status_code), "error")}${row.error_summary ? `<span class="cell-sub">${escapeHtml(row.error_summary)}</span>` : ""}</td><td><span class="cell-main">${formatNumber(row.total_tokens || 0)}</span>${row.cached_input_tokens || row.cache_write_tokens || row.image_input_tokens || row.image_output_tokens || row.reasoning_tokens ? `<span class="cell-sub">缓存读/写 ${formatNumber(row.cached_input_tokens)}/${formatNumber(row.cache_write_tokens)} · 图片 ${formatNumber(row.image_input_tokens)}/${formatNumber(row.image_output_tokens)} · 推理 ${formatNumber(row.reasoning_tokens)}</span>` : ""}</td><td>${formatUsdMicros(row.cost_microusd)}</td>${full ? `<td class="hide-mobile">${escapeHtml(row.request_type || "sync")}${row.service_tier ? `<span class="cell-sub">${escapeHtml(row.service_tier)}</span>` : ""}</td><td class="hide-mobile">${row.duration_ms} ms</td><td class="mono hide-mobile">${escapeHtml(row.request_id.slice(0, 12))}</td><td><button class="button quiet small" data-usage-detail="${row.id}">明细</button></td>` : ""}</tr>`).join("")}</tbody></table></div>`;
}
async function openUsageDetail(event) {
const id = event.currentTarget.dataset.usageDetail;
try {
  const result = await api(`${roleApiBase()}/usage/${id}`);
  const row = result.data;
  openModal("请求明细", `<dl class="detail-list">
    <div><dt>请求 ID</dt><dd class="mono">${escapeHtml(row.request_id)}</dd></div>
    <div><dt>时间</dt><dd>${formatDate(row.created_at)}</dd></div>
    <div><dt>端点</dt><dd class="mono">${escapeHtml(row.endpoint)}</dd></div>
    <div><dt>模型</dt><dd>${escapeHtml(row.model || "-")}</dd></div>
    <div><dt>映射 / 计费模型</dt><dd>${escapeHtml(row.model_mapping_chain || "-")} / ${escapeHtml(row.billing_model || row.model || "-")}</dd></div>
    <div><dt>状态</dt><dd>${row.status_code}</dd></div>
    <div><dt>输入 / 输出 / 总 Token</dt><dd>${formatNumber(row.input_tokens)} / ${formatNumber(row.output_tokens)} / ${formatNumber(row.total_tokens)}</dd></div>
    <div><dt>缓存读 / 写 · 图片入 / 出 · 推理</dt><dd>${formatNumber(row.cached_input_tokens)} / ${formatNumber(row.cache_write_tokens)} · ${formatNumber(row.image_input_tokens)} / ${formatNumber(row.image_output_tokens)} · ${formatNumber(row.reasoning_tokens)}</dd></div>
    <div><dt>请求类型</dt><dd>${escapeHtml(row.request_type || "sync")}${row.service_tier ? ` · ${escapeHtml(row.service_tier)}` : ""}</dd></div>
    <div><dt>成本</dt><dd>${formatUsdMicros(row.cost_microusd)}</dd></div>
    <div><dt>耗时</dt><dd>${formatNumber(row.duration_ms)} ms</dd></div>
    <div><dt>错误摘要</dt><dd>${escapeHtml(row.error_summary || "-")}</dd></div>
  </dl>`, `<button class="button" data-close-modal>关闭</button>`);
} catch (error) { toast(error.message, true); }
}
async function renderSettings(page) {
const [result, mailResult] = await Promise.all([api("/api/admin/settings"), api("/api/admin/mail-settings")]);
const settings = result.data;
const mailSettings = mailResult.data;
page.innerHTML = `
  ${pageHeader("运行设置", "即时生效的网关参数与只读部署信息")}
  <div class="settings-grid">
    <section class="settings-panel">
      <div class="settings-heading"><h2>网关策略</h2><p>修改后对后续请求立即生效。</p></div>
      <form id="runtime-settings-form">
        <div class="field"><label for="setting-site-name">站点名称</label><input id="setting-site-name" name="site_name" value="${escapeHtml(settings.site_name)}" maxlength="80" required></div>
        <div class="field"><label for="setting-site-subtitle">站点副标题</label><input id="setting-site-subtitle" name="site_subtitle" value="${escapeHtml(settings.site_subtitle || "")}" maxlength="200"></div>
        <div class="form-grid"><div class="field"><label for="setting-site-logo">Logo URL 或 data:image</label><input id="setting-site-logo" name="site_logo" value="${escapeHtml(settings.site_logo || "")}" maxlength="262144" placeholder="/logo.svg"></div><div class="field"><label for="setting-doc-url">文档地址</label><input id="setting-doc-url" name="doc_url" type="url" value="${escapeHtml(settings.doc_url || "")}" placeholder="https://docs.example.com"></div></div>
        <div class="field"><label for="setting-contact-info">联系信息</label><input id="setting-contact-info" name="contact_info" value="${escapeHtml(settings.contact_info || "")}" maxlength="500" placeholder="邮箱、群组或支持入口"></div>
        <div class="field"><label for="setting-home-content">首页内容</label><textarea id="setting-home-content" name="home_content" class="page-editor" maxlength="500000" placeholder="Markdown；只填写 HTTP(S) URL 时使用嵌入页面">${escapeHtml(settings.home_content || "")}</textarea><span class="field-hint">Markdown 会在服务端安全渲染；完整 HTTP(S) URL 会显示为嵌入页。</span></div>
        <div class="form-grid"><div class="field"><label for="setting-retries">故障切换次数</label><input id="setting-retries" name="retry_attempts" type="number" min="1" max="5" value="${settings.retry_attempts}" required></div><div class="field"><label for="setting-model-cache">模型缓存秒数</label><input id="setting-model-cache" name="model_cache_seconds" type="number" min="30" max="3600" value="${settings.model_cache_seconds}" required></div></div>
        <div class="form-grid"><div class="field"><label for="setting-5xx-cooldown">5xx 冷却秒数</label><input id="setting-5xx-cooldown" name="cooldown_5xx_seconds" type="number" min="1" max="600" value="${settings.cooldown_5xx_seconds}" required></div><div class="field"><label for="setting-429-cooldown">429 默认冷却秒数</label><input id="setting-429-cooldown" name="cooldown_429_seconds" type="number" min="1" max="3600" value="${settings.cooldown_429_seconds}" required></div></div>
        <div class="field"><label for="setting-audit-retention">审计保留天数</label><input id="setting-audit-retention" name="audit_retention_days" type="number" min="1" max="3650" value="${settings.audit_retention_days}" required></div>
        <div class="settings-heading compact"><h2>界面外观</h2><p>没有保存个人选择的浏览器将使用默认主题。</p></div>
        <div class="field"><span class="field-label">默认主题</span><div class="theme-segmented" role="radiogroup" aria-label="默认主题"><label><input type="radio" name="default_theme" value="light" ${settings.default_theme !== "dark" ? "checked" : ""}><span>${appIcon("sun")}亮色</span></label><label><input type="radio" name="default_theme" value="dark" ${settings.default_theme === "dark" ? "checked" : ""}><span>${appIcon("moon")}暗色</span></label></div><span class="field-hint">侧栏中的主题按钮仍可为当前浏览器单独选择主题。</span></div>
        <div class="settings-heading compact"><h2>账户开放策略</h2><p>验证码和找回邮件使用已选择的邮件传输方式。</p></div>
        <div class="check-row auth-settings">
          <label><input id="setting-registration" type="checkbox" ${settings.registration_enabled ? "checked" : ""}> 开放注册</label>
          <label><input id="setting-email-verification" type="checkbox" ${settings.email_verification_enabled ? "checked" : ""} ${settings.mail_configured ? "" : "disabled"}> 邮箱验证码</label>
          <label><input id="setting-password-reset" type="checkbox" ${settings.password_reset_enabled ? "checked" : ""}> 允许找回密码</label>
          <label><input id="setting-channel-monitor" type="checkbox" ${settings.channel_monitor_enabled ? "checked" : ""}> 频道定时监控</label>
          <label><input id="setting-turnstile" type="checkbox" ${settings.turnstile_enabled ? "checked" : ""}> Turnstile</label>
        </div>
        <div class="form-grid"><div class="field"><label for="setting-turnstile-site">Turnstile Site Key</label><input id="setting-turnstile-site" name="turnstile_site_key" value="${escapeHtml(settings.turnstile_site_key || "")}" maxlength="256"></div><div class="field"><label for="setting-turnstile-secret">Turnstile Secret Key</label><input id="setting-turnstile-secret" name="turnstile_secret_key" type="password" maxlength="512" autocomplete="new-password" placeholder="${settings.turnstile_secret_key_configured ? "留空保留已配置密钥" : "未配置"}"></div></div>
        <div class="field"><label for="setting-monitor-interval">频道监控默认周期（秒）</label><input id="setting-monitor-interval" name="channel_monitor_default_interval_seconds" type="number" min="30" max="86400" value="${settings.channel_monitor_default_interval_seconds}" required></div>
        <p class="field-hint">邮件投递：${settings.mail_configured ? "已配置" : "未配置"}</p>
        <button class="button" type="submit">保存设置</button><p class="form-error" id="settings-error"></p>
      </form>
    </section>
    <section class="settings-panel">
      <div class="settings-heading"><h2>部署信息</h2><p>这些参数来自环境文件，修改后需要重建容器。</p></div>
      <dl class="detail-list"><div><dt>主监听</dt><dd class="mono">${escapeHtml(settings.bind)}</dd></div><div><dt>OAuth 回调</dt><dd class="mono">${escapeHtml(settings.callback_bind)}</dd></div><div><dt>SQLite</dt><dd class="mono">${escapeHtml(settings.database_path)}</dd></div><div><dt>会话时长</dt><dd>${settings.session_hours} 小时</dd></div></dl>
    </section>
    <section class="settings-panel">
      <div class="settings-heading"><h2>邮件投递</h2><p>${mailSettings.webhook_configured ? "Webhook 已由环境文件配置" : "Webhook 未配置"} · SMTP ${mailSettings.smtp_configured ? "已配置" : "未配置"}</p></div>
      <form id="mail-settings-form">
        <div class="form-grid"><div class="field"><label for="mail-mode">传输方式</label><select id="mail-mode" name="mode"><option value="auto" ${mailSettings.mode === "auto" ? "selected" : ""}>自动（优先 Webhook）</option><option value="webhook" ${mailSettings.mode === "webhook" ? "selected" : ""}>仅 Webhook</option><option value="smtp" ${mailSettings.mode === "smtp" ? "selected" : ""}>仅 SMTP</option></select></div><div class="field"><label for="smtp-security">SMTP 安全</label><select id="smtp-security" name="security"><option value="starttls" ${mailSettings.security === "starttls" ? "selected" : ""}>STARTTLS</option><option value="implicit_tls" ${mailSettings.security === "implicit_tls" ? "selected" : ""}>隐式 TLS</option><option value="plain" ${mailSettings.security === "plain" ? "selected" : ""}>明文（仅回环地址）</option></select></div></div>
        <div class="form-grid"><div class="field"><label for="smtp-host">SMTP 主机</label><input id="smtp-host" name="host" value="${escapeHtml(mailSettings.host || "")}" maxlength="253" placeholder="smtp.example.com"></div><div class="field"><label for="smtp-port">端口</label><input id="smtp-port" name="port" type="number" min="1" max="65535" value="${Number(mailSettings.port || 587)}"></div></div>
        <div class="form-grid"><div class="field"><label for="smtp-username">用户名</label><input id="smtp-username" name="username" value="${escapeHtml(mailSettings.username || "")}" maxlength="512" autocomplete="username"></div><div class="field"><label for="smtp-password">密码</label><input id="smtp-password" name="password" type="password" maxlength="4096" autocomplete="new-password" placeholder="${mailSettings.has_password ? "留空保留已保存密码" : "未配置"}"></div></div>
        ${mailSettings.has_password ? '<label class="toggle-line"><input id="smtp-clear-password" type="checkbox"> 清除已保存 SMTP 密码</label>' : ""}
        <div class="form-grid"><div class="field"><label for="smtp-from-email">发件邮箱</label><input id="smtp-from-email" name="from_email" type="email" value="${escapeHtml(mailSettings.from_email || "")}" maxlength="254"></div><div class="field"><label for="smtp-from-name">发件人名称</label><input id="smtp-from-name" name="from_name" value="${escapeHtml(mailSettings.from_name || settings.site_name)}" maxlength="80"></div></div>
        <div class="inline-field"><button class="button" type="submit">保存邮件设置</button><button class="button secondary" id="test-smtp" type="button" ${mailSettings.smtp_configured ? "" : "disabled"}>测试连接</button></div>
        <div class="inline-field"><div class="field"><label for="smtp-test-recipient">测试收件邮箱</label><input id="smtp-test-recipient" type="email" maxlength="254"></div><button class="button secondary" id="send-smtp-test" type="button" ${mailSettings.smtp_configured ? "" : "disabled"}>发送测试邮件</button></div>
        <p class="form-error" id="mail-settings-error"></p>
      </form>
    </section>
  </div>`;
page.querySelector("#runtime-settings-form").addEventListener("submit", saveSettings);
page.querySelector("#mail-settings-form").addEventListener("submit", saveMailSettings);
page.querySelector("#test-smtp").addEventListener("click", testSmtpConnection);
page.querySelector("#send-smtp-test").addEventListener("click", sendSmtpTest);
}
async function saveMailSettings(event) {
event.preventDefault();
const form = event.currentTarget;
const values = Object.fromEntries(new FormData(form));
values.port = Number(values.port || 587);
values.clear_password = Boolean(form.querySelector("#smtp-clear-password")?.checked);
const button = form.querySelector('button[type="submit"]');
const error = form.querySelector("#mail-settings-error");
button.disabled = true; error.textContent = "";
try { await api("/api/admin/mail-settings", { method: "PUT", body: JSON.stringify(values) }); toast("邮件设置已保存"); await renderRoute(); }
catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
}
async function testSmtpConnection(event) {
const button = event.currentTarget; const error = document.querySelector("#mail-settings-error");
button.disabled = true; error.textContent = "";
try { await api("/api/admin/mail-settings/test", { method: "POST", body: "{}" }); toast("SMTP 连接成功"); }
catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
async function sendSmtpTest(event) {
const button = event.currentTarget; const input = document.querySelector("#smtp-test-recipient"); const error = document.querySelector("#mail-settings-error");
if (!input.reportValidity() || !input.value.trim()) return;
button.disabled = true; error.textContent = "";
try { await api("/api/admin/mail-settings/send-test", { method: "POST", body: JSON.stringify({ email: input.value }) }); toast("测试邮件已发送"); }
catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
async function saveSettings(event) {
event.preventDefault();
const form = event.currentTarget;
const values = Object.fromEntries(new FormData(form));
for (const key of ["retry_attempts", "model_cache_seconds", "cooldown_5xx_seconds", "cooldown_429_seconds", "audit_retention_days", "channel_monitor_default_interval_seconds"]) values[key] = Number(values[key]);
values.registration_enabled = form.querySelector("#setting-registration").checked;
values.email_verification_enabled = form.querySelector("#setting-email-verification").checked;
values.password_reset_enabled = form.querySelector("#setting-password-reset").checked;
values.channel_monitor_enabled = form.querySelector("#setting-channel-monitor").checked;
values.turnstile_enabled = form.querySelector("#setting-turnstile").checked;
const button = form.querySelector("button");
const error = form.querySelector("#settings-error");
button.disabled = true; error.textContent = "";
try {
  const result = await api("/api/admin/settings", { method: "PUT", body: JSON.stringify(values) });
  state.siteName = result.data.site_name;
  state.siteSubtitle = result.data.site_subtitle || "个人 AI API 网关";
  state.siteLogo = result.data.site_logo || "/logo.svg";
  state.defaultTheme = normalizeTheme(result.data.default_theme);
  if (!localStorage.getItem(THEME_STORAGE_KEY)) applyTheme(state.defaultTheme);
  state.contactInfo = result.data.contact_info || "";
  state.docUrl = result.data.doc_url || "";
  state.homeContent = result.data.home_content || "";
  state.registrationEnabled = result.data.registration_enabled;
  state.emailVerificationEnabled = result.data.email_verification_enabled;
  state.passwordResetEnabled = result.data.password_reset_enabled;
  state.mailConfigured = result.data.mail_configured;
  state.turnstileEnabled = Boolean(result.data.turnstile_enabled && result.data.turnstile_site_key);
  state.turnstileSiteKey = result.data.turnstile_site_key || "";
  document.title = state.siteName;
  renderShell();
  await renderRoute();
  toast("运行设置已更新");
} catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
async function renderAudit(page) {
const params = new URLSearchParams({ page: String(auditPage), page_size: "30" });
Object.entries(auditFilters).forEach(([key, value]) => { if (value !== "" && value != null) params.set(key, value); });
const result = await api(`/api/admin/audit-logs?${params}`);
const pages = Math.max(1, Math.ceil(result.meta.total / result.meta.page_size));
page.innerHTML = `
  ${pageHeader("审计日志", `${result.meta.total} 条管理操作`, `<button class="button danger" id="clear-audit">清理日志</button>`)}
  <form id="audit-filter-form" class="filter-bar audit-filter">
    <div class="field"><label for="audit-q">操作者或路径</label><input id="audit-q" name="q" value="${escapeHtml(auditFilters.q || "")}" placeholder="搜索"></div>
    <div class="field"><label for="audit-action">操作</label><input id="audit-action" name="action" value="${escapeHtml(auditFilters.action || "")}" placeholder="例如 accounts.put"></div>
    <div class="field"><label for="audit-status">状态码</label><input id="audit-status" name="status_code" type="number" min="100" max="599" value="${escapeHtml(auditFilters.status_code || "")}"></div>
    <div class="field"><label for="audit-start">开始日期</label><input id="audit-start" name="start_date" type="date" value="${escapeHtml(auditFilters.start_date || "")}"></div>
    <div class="field"><label for="audit-end">结束日期</label><input id="audit-end" name="end_date" type="date" value="${escapeHtml(auditFilters.end_date || "")}"></div>
    <div class="filter-actions"><button class="button" type="submit">筛选</button><button class="button secondary" type="button" id="clear-audit-filter">清除</button></div>
  </form>
  ${result.data.length ? auditTable(result.data) : emptyState("暂无审计记录", "管理修改操作会记录在这里")}
  <nav class="pagination" aria-label="审计日志分页"><button class="button secondary" id="audit-prev" ${auditPage <= 1 ? "disabled" : ""}>上一页</button><span>第 ${auditPage} / ${pages} 页</span><button class="button secondary" id="audit-next" ${auditPage >= pages ? "disabled" : ""}>下一页</button></nav>`;
page.querySelector("#audit-filter-form").addEventListener("submit", event => { event.preventDefault(); auditFilters = Object.fromEntries(new FormData(event.currentTarget)); auditPage = 1; renderRoute(); });
page.querySelector("#clear-audit-filter").addEventListener("click", () => { auditFilters = {}; auditPage = 1; renderRoute(); });
page.querySelector("#clear-audit").addEventListener("click", openAuditClear);
page.querySelector("#audit-prev").addEventListener("click", () => { auditPage -= 1; renderRoute(); });
page.querySelector("#audit-next").addEventListener("click", () => { auditPage += 1; renderRoute(); });
page.querySelectorAll("[data-audit-detail]").forEach(button => button.addEventListener("click", openAuditDetail));
}
function openAuditClear() {
openModal("清理审计日志", `<form id="audit-clear-form"><div class="field"><label for="audit-clear-code">管理员 TOTP 动态码</label><input id="audit-clear-code" name="totp_code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code" required autofocus></div><p class="form-error" id="audit-clear-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-audit-clear">清理</button>`);
modal.querySelector("#confirm-audit-clear").addEventListener("click", async event => {
  const form = modal.querySelector("#audit-clear-form"); if (!form.reportValidity()) return; event.currentTarget.disabled = true;
  try { const result = await api("/api/admin/audit-logs/clear", { method: "POST", body: JSON.stringify({ totp_code: form.elements.totp_code.value }) }); closeModal(); toast(`已清理 ${result.data.deleted} 条记录`); auditPage = 1; await renderRoute(); }
  catch (error) { modal.querySelector("#audit-clear-error").textContent = error.message; event.currentTarget.disabled = false; }
});
}
function auditTable(rows) {
return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>操作者</th><th>操作</th><th>结果</th><th>耗时</th><th class="hide-mobile">来源</th><th></th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td><td class="mono">${escapeHtml(row.username)}</td><td><span class="cell-main mono">${escapeHtml(row.action)}</span><span class="cell-sub mono">${escapeHtml(row.method)} ${escapeHtml(row.path)}</span></td><td>${row.status_code < 400 ? status(String(row.status_code)) : status(String(row.status_code), "error")}</td><td>${row.duration_ms} ms</td><td class="mono hide-mobile">${escapeHtml(row.client_ip || "-")}</td><td><button class="button quiet small" data-audit-detail="${row.id}">明细</button></td></tr>`).join("")}</tbody></table></div>`;
}
async function openAuditDetail(event) {
try {
  const result = await api(`/api/admin/audit-logs/${event.currentTarget.dataset.auditDetail}`);
  const row = result.data;
  openModal("审计明细", `<dl class="detail-list"><div><dt>操作</dt><dd class="mono">${escapeHtml(row.action)}</dd></div><div><dt>请求</dt><dd class="mono">${escapeHtml(row.method)} ${escapeHtml(row.path)}</dd></div><div><dt>操作者</dt><dd>${escapeHtml(row.username)}</dd></div><div><dt>状态 / 耗时</dt><dd>${row.status_code} / ${row.duration_ms} ms</dd></div><div><dt>来源 IP</dt><dd class="mono">${escapeHtml(row.client_ip || "-")}</dd></div><div><dt>User-Agent</dt><dd class="mono">${escapeHtml(row.user_agent || "-")}</dd></div><div><dt>请求 ID</dt><dd class="mono">${escapeHtml(row.request_id || "-")}</dd></div><div><dt>时间</dt><dd>${formatDate(row.created_at)}</dd></div></dl>`, `<button class="button" data-close-modal>关闭</button>`);
} catch (error) { toast(error.message, true); }
}
async function renderChannelMonitor(page) {
await loadFeatureScript("content");
return window.Sub2MiniContent.renderMonitor(page);
}
function monitorStatus(value) {
if (value === "operational") return status("正常");
if (value === "degraded") return status("降级", "warn");
if (value === "failed" || value === "error") return status("异常", "error");
return status("未检测", "off");
}
function monitorStatusText(value) {
return ({ operational: "正常", degraded: "降级", failed: "失败", error: "错误" })[value] || "未检测";
}
async function renderChannelMonitorAdmin(page) {
await loadFeatureScript("monitor-admin");
return window.Sub2MiniMonitorAdmin.render(page);
}
function monitorAdminTable(items) {
return `<div class="table-wrap"><table><thead><tr><th>名称</th><th>提供方</th><th>主模型</th><th>状态</th><th>7 天可用率</th><th>延迟</th><th>周期</th><th></th></tr></thead><tbody>${items.map(item => `<tr><td><span class="cell-main">${escapeHtml(item.name)}</span><span class="cell-sub">${escapeHtml(item.group_name || item.endpoint)}</span></td><td>${escapeHtml(item.provider.toUpperCase())}</td><td><span class="cell-main mono">${escapeHtml(item.primary_model)}</span><span class="cell-sub">${item.extra_models.length} 个附加模型</span></td><td>${monitorStatus(item.primary_status)}${item.enabled ? "" : `<span class="cell-sub">监控已停用</span>`}</td><td>${Number(item.availability_7d).toFixed(2)}%</td><td>${item.primary_latency_ms == null ? "-" : `${item.primary_latency_ms} ms`}</td><td>${item.interval_seconds} 秒</td><td><div class="cell-actions"><button class="button quiet small" data-monitor-action="run" data-id="${item.id}">运行</button><button class="button quiet small" data-monitor-action="history" data-id="${item.id}">历史</button><button class="button quiet small" data-monitor-action="duplicate" data-id="${item.id}">复制</button><button class="button quiet small" data-monitor-action="edit" data-id="${item.id}">编辑</button><button class="button quiet small" data-monitor-action="toggle" data-id="${item.id}">${item.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-monitor-action="delete" data-id="${item.id}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
}
function openMonitorModal(item = null) {
openModal(item ? "编辑频道监控" : "创建频道监控", `<form id="monitor-form">
  <div class="form-grid"><div class="field"><label for="monitor-name">名称</label><input id="monitor-name" name="name" value="${escapeHtml(item?.name || "")}" maxlength="100" required autofocus></div><div class="field"><label for="monitor-group">分组</label><input id="monitor-group" name="group_name" value="${escapeHtml(item?.group_name || "")}" maxlength="80"></div></div>
  <div class="form-grid"><div class="field"><label for="monitor-provider">提供方</label><select id="monitor-provider" name="provider">${["openai", "anthropic", "gemini", "grok"].map(value => `<option value="${value}" ${item?.provider === value ? "selected" : ""}>${value.toUpperCase()}</option>`).join("")}</select></div><div class="field"><label for="monitor-mode">API 模式</label><select id="monitor-mode" name="api_mode"><option value="chat_completions" ${item?.api_mode !== "responses" ? "selected" : ""}>Chat Completions</option><option value="responses" ${item?.api_mode === "responses" ? "selected" : ""}>Responses</option></select></div></div>
  <div class="field"><label for="monitor-endpoint">探测端点</label><input id="monitor-endpoint" name="endpoint" type="url" value="${escapeHtml(item?.endpoint || "https://api.openai.com/v1/chat/completions")}" required></div>
  <div class="field"><label for="monitor-api-key">API Key</label><input id="monitor-api-key" name="api_key" type="password" ${item ? "" : "required"} autocomplete="new-password"><span class="field-hint">${item ? `留空保留 ${escapeHtml(item.api_key_masked)}` : "加密保存，不会在列表或日志中返回"}</span></div>
  <div class="field"><label for="monitor-primary-model">主模型</label><input id="monitor-primary-model" name="primary_model" value="${escapeHtml(item?.primary_model || "")}" required></div>
  <div class="field"><label for="monitor-extra-models">附加模型</label><textarea id="monitor-extra-models" name="extra_models" class="compact-textarea" placeholder="每行一个模型">${escapeHtml((item?.extra_models || []).join("\n"))}</textarea></div>
  <div class="form-grid"><div class="field"><label for="monitor-interval">检查周期（秒）</label><input id="monitor-interval" name="interval_seconds" type="number" min="30" max="86400" value="${item?.interval_seconds || 300}" required></div><div class="field"><label for="monitor-jitter">随机偏移（秒）</label><input id="monitor-jitter" name="jitter_seconds" type="number" min="0" max="3600" value="${item?.jitter_seconds || 0}" required></div></div>
  <div class="field"><label for="monitor-headers">附加请求头（JSON）</label><textarea id="monitor-headers" name="extra_headers" class="compact-textarea" spellcheck="false">${escapeHtml(JSON.stringify(item?.extra_headers || {}, null, 2))}</textarea></div>
  <div class="field"><label for="monitor-override-mode">请求体覆盖</label><select id="monitor-override-mode" name="body_override_mode"><option value="off" ${!item || item.body_override_mode === "off" ? "selected" : ""}>关闭</option><option value="merge" ${item?.body_override_mode === "merge" ? "selected" : ""}>合并</option><option value="replace" ${item?.body_override_mode === "replace" ? "selected" : ""}>替换</option></select></div>
  <div class="field"><label for="monitor-body">请求体覆盖（JSON）</label><textarea id="monitor-body" name="body_override" class="compact-textarea" spellcheck="false">${item?.body_override ? escapeHtml(JSON.stringify(item.body_override, null, 2)) : ""}</textarea></div>
  <label class="switch-row"><span><strong>启用定时监控</strong><small>同一 Rust 进程按周期执行</small></span><input name="enabled" type="checkbox" ${item?.enabled === false ? "" : "checked"}></label>
  <p class="form-error" id="monitor-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-monitor">保存</button>`);
modal.querySelector("#save-monitor").addEventListener("click", () => saveMonitor(item?.id));
}
async function saveMonitor(id = null) {
const form = modal.querySelector("#monitor-form");
if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
try {
  values.extra_models = parseModelList(values.extra_models);
  values.interval_seconds = Number(values.interval_seconds); values.jitter_seconds = Number(values.jitter_seconds);
  values.enabled = form.elements.enabled.checked;
  values.extra_headers = JSON.parse(values.extra_headers || "{}");
  values.body_override = values.body_override.trim() ? JSON.parse(values.body_override) : null;
  if (id && !values.api_key) delete values.api_key;
  const button = modal.querySelector("#save-monitor"); button.disabled = true;
  await api(id ? `/api/admin/channel-monitors/${id}` : "/api/admin/channel-monitors", { method: id ? "PUT" : "POST", body: JSON.stringify(values) });
  closeModal(); toast(id ? "监控已更新" : "监控已创建"); await renderRoute();
} catch (error) { modal.querySelector("#monitor-error").textContent = error.message || "JSON 格式无效"; modal.querySelector("#save-monitor").disabled = false; }
}
async function handleMonitorAction(event) {
const button = event.currentTarget;
const item = currentMonitors.find(value => String(value.id) === String(button.dataset.id));
if (!item) return;
const action = button.dataset.monitorAction;
if (action === "edit") return openMonitorModal(item);
if (action === "delete") {
  openModal("删除频道监控", `<p>确认删除 <strong>${escapeHtml(item.name)}</strong> 及全部历史？</p><p class="form-error" id="monitor-delete-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-delete-monitor">删除</button>`);
  modal.querySelector("#confirm-delete-monitor").addEventListener("click", async event => { event.currentTarget.disabled = true; try { await api(`/api/admin/channel-monitors/${item.id}`, { method: "DELETE" }); closeModal(); toast("监控已删除"); await renderRoute(); } catch (error) { modal.querySelector("#monitor-delete-error").textContent = error.message; event.currentTarget.disabled = false; } });
  return;
}
button.disabled = true;
try {
  if (action === "run") {
    const result = await api(`/api/admin/channel-monitors/${item.id}/run`, { method: "POST", body: "{}" });
    openMonitorRunResults(item.name, result.data.results);
    return;
  }
  if (action === "history") {
    const result = await api(`/api/admin/channel-monitors/${item.id}/history?limit=100`);
    openMonitorHistory(item.name, result.data);
    return;
  }
  if (action === "duplicate") await api(`/api/admin/channel-monitors/${item.id}/duplicate`, { method: "POST", body: "{}" });
  if (action === "toggle") await api(`/api/admin/channel-monitors/${item.id}`, { method: "PUT", body: JSON.stringify({ enabled: !item.enabled }) });
  toast(action === "duplicate" ? "监控副本已创建并默认停用" : "监控状态已更新"); await renderRoute();
} catch (error) { toast(error.message, true); }
finally { button.disabled = false; }
}
function openMonitorRunResults(name, rows) {
openModal(`${name} · 探测结果`, monitorHistoryTable(rows), `<button class="button" data-close-modal>关闭</button>`);
}
function openMonitorHistory(name, rows) {
openModal(`${name} · 历史`, rows.length ? monitorHistoryTable(rows) : emptyState("暂无探测历史", "运行一次监控后会显示结果"), `<button class="button" data-close-modal>关闭</button>`);
}
function monitorHistoryTable(rows) {
return `<div class="table-wrap"><table><thead><tr><th>时间</th><th>模型</th><th>状态</th><th>延迟</th><th>说明</th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.checked_at)}</td><td class="mono">${escapeHtml(row.model)}</td><td>${monitorStatus(row.status)}</td><td>${row.latency_ms == null ? "-" : `${row.latency_ms} ms`}</td><td>${escapeHtml(row.message || "-")}</td></tr>`).join("")}</tbody></table></div>`;
}
async function renderStatus(page) {
const started = performance.now();
const health = await fetch(`${API}/health`).then(response => response.json());
const latency = Math.round(performance.now() - started);
page.innerHTML = `
  ${pageHeader("服务状态", "当前运行实例")}
  <section class="metric-grid">
    ${metric("API", health.status === "ok" ? "正常" : "异常", health.status === "ok" ? "good" : "warn")}
    ${metric("版本", health.version)}
    ${metric("响应时间", `${latency} ms`)}
  </section>
  <section class="section"><div class="table-wrap"><table><tbody>
    <tr><th>管理页面</th><td class="mono">${escapeHtml(location.origin)}</td></tr>
    <tr><th>API Base URL</th><td class="mono">${escapeHtml(API)}/v1</td></tr>
    <tr><th>服务</th><td>${escapeHtml(health.service)}</td></tr>
  </tbody></table></div></section>`;
}
async function logout() {
try { await api("/api/auth/logout", { method: "POST", body: "{}" }); } catch (_) {}
sessionStorage.removeItem("mini_csrf");
window.Sub2MiniEngagement?.reset();
renderLogin();
}
async function renderRiskControlAdmin(page) {
const params = new URLSearchParams({ page: String(riskLogPage), page_size: "20" });
Object.entries(riskLogFilters).forEach(([key, value]) => { if (value !== "" && value != null) params.set(key, value); });
const [configResult, statusResult, logsResult, groupsResult] = await Promise.all([
  api("/api/admin/risk-control/config"),
  api("/api/admin/risk-control/status"),
  api(`/api/admin/risk-control/logs?${params}`),
  api("/api/admin/groups"),
]);
const config = configResult.data;
const runtime = statusResult.data;
const logs = logsResult.data;
currentRiskLogs = logs.items;
currentRiskGroups = groupsResult.data;
const selectedGroups = new Set(config.group_ids || []);
const modelFilter = config.model_filter || { type: "all", models: [] };
const thresholds = Object.entries(config.thresholds || {});
page.innerHTML = `
  ${pageHeader("风险控制", "在请求进入上游前执行内容策略；输入正文不会写入数据库", `<button class="button secondary" id="risk-test">测试审核</button><button class="button" id="risk-save">保存策略</button>`)}
  <div class="metric-grid risk-metrics">
    ${metric("运行状态", config.enabled && config.mode !== "off" ? (config.mode === "pre_block" ? "预拦截" : "观察") : "停用", config.enabled ? "good" : "")}
    ${metric("累计检查", formatNumber(runtime.processed))}
    ${metric("已拦截", formatNumber(runtime.pre_block_blocked), runtime.pre_block_blocked ? "warn" : "good")}
    ${metric("审核错误", formatNumber(runtime.errors), runtime.errors ? "warn" : "good")}
    ${metric("命中哈希", formatNumber(runtime.flagged_hash_count))}
    ${metric("审核密钥", formatNumber(config.api_key_count))}
  </div>
  <form id="risk-config-form" class="risk-config">
    <section class="risk-section">
      <div class="settings-heading"><h2>基础策略</h2><p>观察模式只记录命中；预拦截模式在转发前返回配置的错误。</p></div>
      <div class="form-grid"><label class="switch-row"><span><strong>启用风险控制</strong><small>关闭后跳过所有内容检查</small></span><input id="risk-enabled" type="checkbox" ${config.enabled ? "checked" : ""}></label><div class="field"><label for="risk-mode">模式</label><select id="risk-mode"><option value="pre_block" ${config.mode === "pre_block" ? "selected" : ""}>预拦截</option><option value="observe" ${config.mode === "observe" ? "selected" : ""}>仅观察</option><option value="off" ${config.mode === "off" ? "selected" : ""}>关闭</option></select></div></div>
      <div class="form-grid"><div class="field"><label for="risk-sample">采样比例 (%)</label><input id="risk-sample" type="number" min="0" max="100" value="${config.sample_rate}" required></div><div class="field"><label for="risk-keyword-mode">检查策略</label><select id="risk-keyword-mode"><option value="keyword_and_api" ${config.keyword_blocking_mode === "keyword_and_api" ? "selected" : ""}>关键词 + 审核 API</option><option value="keyword_only" ${config.keyword_blocking_mode === "keyword_only" ? "selected" : ""}>仅关键词</option><option value="api_only" ${config.keyword_blocking_mode === "api_only" ? "selected" : ""}>仅审核 API</option></select></div></div>
    </section>
    <section class="risk-section">
      <div class="settings-heading"><h2>审核上游</h2><p>支持 OpenAI 兼容的 <span class="mono">/v1/moderations</span> 接口，密钥加密保存。</p></div>
      <div class="form-grid"><div class="field"><label for="risk-base-url">Base URL</label><input id="risk-base-url" type="url" value="${escapeHtml(config.base_url)}" required></div><div class="field"><label for="risk-model">审核模型</label><input id="risk-model" value="${escapeHtml(config.model)}" maxlength="128" required></div></div>
      <div class="form-grid"><div class="field"><label for="risk-timeout">超时 (ms)</label><input id="risk-timeout" type="number" min="500" max="30000" value="${config.timeout_ms}" required></div><div class="field"><label for="risk-retries">重试次数</label><input id="risk-retries" type="number" min="0" max="5" value="${config.retry_count}" required></div></div>
      <div class="field"><label for="risk-api-keys">新增审核 API Key</label><textarea id="risk-api-keys" class="compact-textarea" autocomplete="off" placeholder="每行一个；留空保留现有密钥"></textarea></div>
      <div class="inline-field"><div class="field"><label for="risk-key-mode">写入方式</label><select id="risk-key-mode"><option value="append">追加</option><option value="replace">替换全部</option></select></div><label class="toggle-line"><input id="risk-clear-keys" type="checkbox"> 清除全部已保存密钥</label></div>
      <div class="risk-key-list">${config.api_key_statuses.length ? config.api_key_statuses.map(item => `<div><span class="mono">${escapeHtml(item.masked)}</span>${riskKeyStatus(item)}<small>${riskKeyStatusMeta(item)}</small><button class="button quiet small" type="button" data-risk-delete-key="${escapeHtml(item.key_hash)}">移除</button></div>`).join("") : '<span class="field-hint">尚未配置审核 API Key；仅关键词模式仍可使用。</span>'}</div>
      ${runtime.pre_block_api_key_loads?.length ? `<div class="table-wrap risk-key-loads"><table><thead><tr><th>密钥</th><th>当前负载</th><th>累计</th><th>成功 / 错误</th><th>平均 / 最近耗时</th></tr></thead><tbody>${runtime.pre_block_api_key_loads.map(item => `<tr><td class="mono">${escapeHtml(item.masked)}</td><td>${formatNumber(item.active)}</td><td>${formatNumber(item.total)}</td><td>${formatNumber(item.success)} / ${formatNumber(item.errors)}</td><td>${formatNumber(item.avg_latency_ms)} / ${formatNumber(item.last_latency_ms)} ms</td></tr>`).join("")}</tbody></table></div>` : ""}
    </section>
    <section class="risk-section">
      <div class="settings-heading"><h2>检查范围</h2><p>按路由分组和模型控制审计范围。</p></div>
      <label class="switch-row compact"><span><strong>全部路由分组</strong><small>关闭后仅检查下方选中的分组</small></span><input id="risk-all-groups" type="checkbox" ${config.all_groups ? "checked" : ""}></label>
      <div class="choice-grid risk-group-list">${currentRiskGroups.map(group => `<label><input type="checkbox" name="risk_group_id" value="${group.id}" ${selectedGroups.has(group.id) ? "checked" : ""}><span>${escapeHtml(group.name)}</span><small>${group.account_ids.length} 个账号</small></label>`).join("") || '<span class="field-hint">暂无路由分组</span>'}</div>
      <div class="form-grid"><div class="field"><label for="risk-model-filter">模型范围</label><select id="risk-model-filter"><option value="all" ${modelFilter.type === "all" ? "selected" : ""}>全部模型</option><option value="include" ${modelFilter.type === "include" ? "selected" : ""}>仅包含列表</option><option value="exclude" ${modelFilter.type === "exclude" ? "selected" : ""}>排除列表</option></select></div><div class="field"><label for="risk-model-list">模型列表</label><textarea id="risk-model-list" class="compact-textarea" placeholder="每行一个模型 ID">${escapeHtml((modelFilter.models || []).join("\n"))}</textarea></div></div>
    </section>
    <section class="risk-section">
      <div class="settings-heading"><h2>本地规则与阈值</h2><p>关键词不区分大小写；阈值用于覆盖审核 API 返回的分类分数。</p></div>
      <div class="field"><label for="risk-keywords">拦截关键词</label><textarea id="risk-keywords" class="risk-keywords" placeholder="每行一个关键词">${escapeHtml((config.blocked_keywords || []).join("\n"))}</textarea></div>
      <div class="risk-threshold-grid">${thresholds.map(([category, value]) => `<div class="field"><label>${escapeHtml(category)}</label><input name="risk_threshold" data-category="${escapeHtml(category)}" type="number" min="0" max="1" step="0.01" value="${Number(value)}"></div>`).join("")}</div>
    </section>
    <section class="risk-section">
      <div class="settings-heading"><h2>响应与自动封禁</h2><p>管理员账号永远不会被自动封禁。</p></div>
      <div class="form-grid"><div class="field"><label for="risk-block-status">拦截状态码</label><input id="risk-block-status" type="number" min="400" max="599" value="${config.block_status}" required></div><div class="field"><label for="risk-block-message">客户端消息</label><input id="risk-block-message" value="${escapeHtml(config.block_message)}" maxlength="500" required></div></div>
      <div class="form-grid"><label class="switch-row compact"><span><strong>自动封禁普通用户</strong><small>命中次数达到阈值后禁用账户</small></span><input id="risk-auto-ban" type="checkbox" ${config.auto_ban_enabled ? "checked" : ""}></label><label class="switch-row compact"><span><strong>记录未命中请求</strong><small>仅保存摘要，不保存输入正文</small></span><input id="risk-record-pass" type="checkbox" ${config.record_non_hits ? "checked" : ""}></label></div>
      <div class="form-grid"><div class="field"><label for="risk-ban-threshold">封禁阈值</label><input id="risk-ban-threshold" type="number" min="1" max="1000" value="${config.ban_threshold}" required></div><div class="field"><label for="risk-ban-window">统计窗口 (小时)</label><input id="risk-ban-window" type="number" min="1" max="8760" value="${config.violation_window_hours}" required></div></div>
      <div class="form-grid"><label class="switch-row compact"><span><strong>命中哈希预检</strong><small>重复内容无需再次调用审核 API</small></span><input id="risk-prehash" type="checkbox" ${config.pre_hash_check_enabled ? "checked" : ""}></label><label class="switch-row compact"><span><strong>命中邮件通知</strong><small>${config.mail_configured ? "通知触发用户的已绑定邮箱；自动封禁始终通知" : "邮件投递未配置，命中仍会正常记录"}</small></span><input id="risk-email-hit" type="checkbox" ${config.email_on_hit ? "checked" : ""}></label></div>
    </section>
    <section class="risk-section">
      <div class="settings-heading"><h2>运行与保留</h2><p>Mini 使用同进程异步任务，队列参数用于兼容配置和容量约束。</p></div>
      <div class="risk-threshold-grid"><div class="field"><label for="risk-workers">工作槽</label><input id="risk-workers" type="number" min="1" max="32" value="${config.worker_count}" required></div><div class="field"><label for="risk-queue">容量</label><input id="risk-queue" type="number" min="100" max="100000" value="${config.queue_size}" required></div><div class="field"><label for="risk-hit-days">命中保留天数</label><input id="risk-hit-days" type="number" min="1" max="3650" value="${config.hit_retention_days}" required></div><div class="field"><label for="risk-pass-days">未命中保留天数</label><input id="risk-pass-days" type="number" min="1" max="365" value="${config.non_hit_retention_days}" required></div></div>
      <div class="actions"><button class="button danger" id="risk-clear-hashes" type="button" ${runtime.flagged_hash_count ? "" : "disabled"}>清除 ${formatNumber(runtime.flagged_hash_count)} 个命中哈希</button></div>
    </section>
    <p class="form-error" id="risk-config-error"></p>
  </form>
  <section class="section risk-log-section">
    <div class="section-title"><div><h2>检查记录</h2><p>${formatNumber(logs.total)} 条记录</p></div><button class="button secondary small" id="risk-refresh">刷新</button></div>
    <form id="risk-log-filter" class="filter-bar risk-filter">
      <div class="field"><label for="risk-result">结果</label><select id="risk-result" name="result"><option value="">全部</option><option value="blocked" ${riskLogFilters.result === "blocked" ? "selected" : ""}>已拦截</option><option value="hit" ${riskLogFilters.result === "hit" ? "selected" : ""}>命中</option><option value="pass" ${riskLogFilters.result === "pass" ? "selected" : ""}>通过</option><option value="error" ${riskLogFilters.result === "error" ? "selected" : ""}>错误</option></select></div>
      <div class="field"><label for="risk-endpoint">端点</label><select id="risk-endpoint" name="endpoint"><option value="">全部</option><option value="/v1/responses" ${riskLogFilters.endpoint === "/v1/responses" ? "selected" : ""}>/v1/responses</option><option value="/v1/chat/completions" ${riskLogFilters.endpoint === "/v1/chat/completions" ? "selected" : ""}>/v1/chat/completions</option></select></div>
      <div class="field"><label for="risk-search">搜索</label><input id="risk-search" name="search" value="${escapeHtml(riskLogFilters.search || "")}" placeholder="用户、Key、模型、请求 ID"></div>
      <div class="field"><label for="risk-from">开始</label><input id="risk-from" name="from" type="datetime-local" value="${escapeHtml(riskLogFilters.from || "")}"></div>
      <div class="field"><label for="risk-to">结束</label><input id="risk-to" name="to" type="datetime-local" value="${escapeHtml(riskLogFilters.to || "")}"></div>
      <div class="filter-actions"><button class="button" type="submit">筛选</button><button class="button secondary" id="risk-filter-clear" type="button">清除</button></div>
    </form>
    ${logs.items.length ? riskLogTable(logs.items) : emptyState("暂无风险记录", "命中、本地错误或启用未命中记录后会显示")}
    <nav class="pagination"><button class="button secondary" id="risk-prev" ${logs.page <= 1 ? "disabled" : ""}>上一页</button><span>第 ${logs.page} / ${logs.pages} 页</span><button class="button secondary" id="risk-next" ${logs.page >= logs.pages ? "disabled" : ""}>下一页</button></nav>
  </section>`;
page.querySelector("#risk-save").addEventListener("click", saveRiskControl);
page.querySelector("#risk-test").addEventListener("click", openRiskTest);
page.querySelector("#risk-refresh").addEventListener("click", renderRoute);
page.querySelector("#risk-clear-hashes").addEventListener("click", openRiskHashClear);
page.querySelectorAll("[data-risk-delete-key]").forEach(button => button.addEventListener("click", removeRiskKey));
page.querySelectorAll("[data-risk-log]").forEach(button => button.addEventListener("click", openRiskLog));
page.querySelectorAll("[data-risk-unban]").forEach(button => button.addEventListener("click", unbanRiskUser));
page.querySelector("#risk-log-filter").addEventListener("submit", event => { event.preventDefault(); riskLogFilters = Object.fromEntries(new FormData(event.currentTarget)); riskLogPage = 1; renderRoute(); });
page.querySelector("#risk-filter-clear").addEventListener("click", () => { riskLogFilters = {}; riskLogPage = 1; renderRoute(); });
page.querySelector("#risk-prev").addEventListener("click", () => { riskLogPage -= 1; renderRoute(); });
page.querySelector("#risk-next").addEventListener("click", () => { riskLogPage += 1; renderRoute(); });
}
function riskLogTable(rows) {
return `<div class="table-wrap"><table class="risk-table"><thead><tr><th>时间</th><th>用户 / Key</th><th>端点 / 模型</th><th>结果</th><th>分类</th><th>命中次数</th><th></th></tr></thead><tbody>${rows.map(row => `<tr><td>${formatDate(row.created_at)}</td><td><span class="cell-main">${escapeHtml(row.user_email || "-")}</span><span class="cell-sub">${escapeHtml(row.api_key_name || "-")}</span></td><td><span class="cell-main mono">${escapeHtml(row.endpoint)}</span><span class="cell-sub mono">${escapeHtml(row.model || "-")}</span></td><td>${row.action === "blocked" ? status("已拦截", "error") : row.action === "hit" ? status("命中", "warn") : row.action === "error" ? status("错误", "error") : status("通过")}</td><td><span class="cell-main">${escapeHtml(row.highest_category || "-")}</span>${row.matched_keyword ? `<span class="cell-sub">关键词：${escapeHtml(row.matched_keyword)}</span>` : ""}</td><td>${row.violation_count || 0}${row.auto_banned ? '<span class="cell-sub">已自动封禁</span>' : ""}</td><td><div class="cell-actions">${row.user_id && row.user_status === "disabled" ? `<button class="button quiet small" data-risk-unban="${row.user_id}">解封</button>` : ""}<button class="button quiet small" data-risk-log="${row.id}">详情</button></div></td></tr>`).join("")}</tbody></table></div>`;
}
function riskKeyStatus(item) {
if (item.status === "frozen") return status("已冻结", "error");
if (item.status === "ok") return status("正常");
if (item.status === "error") return status("异常", "warn");
return status("未测试", "off");
}
function riskKeyStatusMeta(item) {
const parts = [`成功 ${formatNumber(item.success_count || 0)}`, `连续失败 ${formatNumber(item.failure_count || 0)}`];
if (item.last_latency_ms > 0) parts.push(`${formatNumber(item.last_latency_ms)} ms`);
if (item.last_http_status > 0) parts.push(`HTTP ${item.last_http_status}`);
if (item.frozen_until && item.status === "frozen") parts.push(`冻结至 ${formatDate(item.frozen_until)}`);
else if (item.last_checked_at) parts.push(`检查于 ${formatDate(item.last_checked_at)}`);
return escapeHtml(parts.join(" · "));
}
async function saveRiskControl() {
const form = document.querySelector("#risk-config-form");
if (!form.reportValidity()) return;
const button = document.querySelector("#risk-save");
const error = form.querySelector("#risk-config-error");
const thresholds = {};
form.querySelectorAll('[name="risk_threshold"]').forEach(input => { thresholds[input.dataset.category] = Number(input.value); });
const payload = {
  enabled: form.querySelector("#risk-enabled").checked,
  mode: form.querySelector("#risk-mode").value,
  base_url: form.querySelector("#risk-base-url").value,
  model: form.querySelector("#risk-model").value,
  api_keys: parseModelList(form.querySelector("#risk-api-keys").value),
  api_keys_mode: form.querySelector("#risk-key-mode").value,
  clear_api_key: form.querySelector("#risk-clear-keys").checked,
  timeout_ms: Number(form.querySelector("#risk-timeout").value),
  retry_count: Number(form.querySelector("#risk-retries").value),
  sample_rate: Number(form.querySelector("#risk-sample").value),
  all_groups: form.querySelector("#risk-all-groups").checked,
  group_ids: [...form.querySelectorAll('[name="risk_group_id"]:checked')].map(input => Number(input.value)),
  model_filter: { type: form.querySelector("#risk-model-filter").value, models: parseModelList(form.querySelector("#risk-model-list").value) },
  blocked_keywords: parseModelList(form.querySelector("#risk-keywords").value),
  keyword_blocking_mode: form.querySelector("#risk-keyword-mode").value,
  thresholds,
  block_status: Number(form.querySelector("#risk-block-status").value),
  block_message: form.querySelector("#risk-block-message").value,
  auto_ban_enabled: form.querySelector("#risk-auto-ban").checked,
  record_non_hits: form.querySelector("#risk-record-pass").checked,
  ban_threshold: Number(form.querySelector("#risk-ban-threshold").value),
  violation_window_hours: Number(form.querySelector("#risk-ban-window").value),
  pre_hash_check_enabled: form.querySelector("#risk-prehash").checked,
  email_on_hit: form.querySelector("#risk-email-hit").checked,
  worker_count: Number(form.querySelector("#risk-workers").value),
  queue_size: Number(form.querySelector("#risk-queue").value),
  hit_retention_days: Number(form.querySelector("#risk-hit-days").value),
  non_hit_retention_days: Number(form.querySelector("#risk-pass-days").value),
};
button.disabled = true; error.textContent = "";
try { await api("/api/admin/risk-control/config", { method: "PUT", body: JSON.stringify(payload) }); toast("风险策略已保存"); await renderRoute(); }
catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
function openRiskTest() {
openModal("测试审核接口", `<form id="risk-test-form"><div class="field"><label for="risk-test-keys">临时 API Key（可选）</label><textarea id="risk-test-keys" name="api_keys" class="compact-textarea" placeholder="留空使用已保存密钥"></textarea></div><div class="field"><label for="risk-test-prompt">测试文本</label><textarea id="risk-test-prompt" name="prompt" required>这是一次审核连通性测试。</textarea></div><div id="risk-test-result"></div><p class="form-error" id="risk-test-error"></p></form>`, `<button class="button secondary" data-close-modal>关闭</button><button class="button" id="run-risk-test">运行测试</button>`);
modal.querySelector("#run-risk-test").addEventListener("click", async event => {
  const form = modal.querySelector("#risk-test-form"); if (!form.reportValidity()) return;
  event.currentTarget.disabled = true; form.querySelector("#risk-test-error").textContent = "";
  try {
    const values = Object.fromEntries(new FormData(form)); values.api_keys = parseModelList(values.api_keys);
    const result = await api("/api/admin/risk-control/api-keys/test", { method: "POST", body: JSON.stringify(values) });
    form.querySelector("#risk-test-result").innerHTML = `<div class="risk-test-results">${result.data.items.map(item => `<div><span class="mono">${escapeHtml(item.masked)}</span>${item.status === "ok" ? status("可用") : status("失败", "error")}<small>${item.last_latency_ms} ms${item.last_error ? ` · ${escapeHtml(item.last_error)}` : ""}</small></div>`).join("")}</div>${result.data.audit_result ? `<p class="field-hint">审核结果：${result.data.audit_result.flagged ? "命中" : "通过"} · ${escapeHtml(result.data.audit_result.highest_category || "无分类")} · ${Number(result.data.audit_result.highest_score || 0).toFixed(4)}</p>` : ""}`;
  } catch (error) { form.querySelector("#risk-test-error").textContent = error.message; }
  finally { event.currentTarget.disabled = false; }
});
}
function openRiskHashClear() {
openModal("清除命中哈希", "<p>清除后，重复内容将重新执行完整检查。风险日志不会删除。</p><p class=\"form-error\" id=\"risk-hash-error\"></p>", `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-risk-hash-clear">清除</button>`);
modal.querySelector("#confirm-risk-hash-clear").addEventListener("click", async event => { event.currentTarget.disabled = true; try { const result = await api("/api/admin/risk-control/hashes/all", { method: "DELETE" }); closeModal(); toast(`已清除 ${result.data.deleted} 个哈希`); await renderRoute(); } catch (error) { modal.querySelector("#risk-hash-error").textContent = error.message; event.currentTarget.disabled = false; } });
}
function removeRiskKey(event) {
const hash = event.currentTarget.dataset.riskDeleteKey;
openModal("移除审核密钥", "<p>密钥明文无法恢复，确认移除这个审核 API Key？</p><p class=\"form-error\" id=\"risk-key-error\"></p>", `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-risk-key-delete">移除</button>`);
modal.querySelector("#confirm-risk-key-delete").addEventListener("click", async click => { click.currentTarget.disabled = true; try { await api("/api/admin/risk-control/config", { method: "PUT", body: JSON.stringify({ delete_api_key_hashes: [hash] }) }); closeModal(); toast("审核密钥已移除"); await renderRoute(); } catch (error) { modal.querySelector("#risk-key-error").textContent = error.message; click.currentTarget.disabled = false; } });
}
function openRiskLog(event) {
const row = currentRiskLogs.find(item => String(item.id) === String(event.currentTarget.dataset.riskLog)); if (!row) return;
const scores = Object.entries(row.category_scores || {}).sort((left, right) => Number(right[1]) - Number(left[1]));
openModal("风险检查详情", `<dl class="detail-list"><div><dt>请求 ID</dt><dd class="mono">${escapeHtml(row.request_id)}</dd></div><div><dt>时间</dt><dd>${formatDate(row.created_at)}</dd></div><div><dt>模式 / 动作</dt><dd>${escapeHtml(row.mode)} / ${escapeHtml(row.action)}</dd></div><div><dt>用户 / API Key</dt><dd>${escapeHtml(row.user_email || "-")} / ${escapeHtml(row.api_key_name || "-")}</dd></div><div><dt>端点 / 模型</dt><dd class="mono">${escapeHtml(row.endpoint)} / ${escapeHtml(row.model || "-")}</dd></div><div><dt>输入 SHA-256</dt><dd class="mono">${escapeHtml(row.input_hash || "-")}</dd></div><div><dt>最高分类</dt><dd>${escapeHtml(row.highest_category || "-")} · ${Number(row.highest_score || 0).toFixed(4)}</dd></div><div><dt>关键词</dt><dd>${escapeHtml(row.matched_keyword || "-")}</dd></div><div><dt>审核耗时</dt><dd>${row.upstream_latency_ms == null ? "-" : `${row.upstream_latency_ms} ms`}</dd></div><div><dt>邮件通知</dt><dd>${row.email_sent ? "已发送" : "未发送"}</dd></div><div><dt>错误摘要</dt><dd>${escapeHtml(row.error || "-")}</dd></div></dl>${scores.length ? `<div class="risk-score-list">${scores.map(([category, value]) => `<div><span>${escapeHtml(category)}</span><strong>${Number(value).toFixed(4)}</strong></div>`).join("")}</div>` : ""}`, `<button class="button secondary" data-close-modal>关闭</button>${row.input_hash ? '<button class="button danger" id="delete-risk-hash">删除此哈希</button>' : ""}`);
modal.querySelector("#delete-risk-hash")?.addEventListener("click", async click => { click.currentTarget.disabled = true; try { await api("/api/admin/risk-control/hashes", { method: "DELETE", body: JSON.stringify({ input_hash: row.input_hash }) }); closeModal(); toast("命中哈希已删除"); await renderRoute(); } catch (error) { toast(error.message, true); click.currentTarget.disabled = false; } });
}
async function unbanRiskUser(event) {
event.currentTarget.disabled = true;
try { await api(`/api/admin/risk-control/users/${event.currentTarget.dataset.riskUnban}/unban`, { method: "POST", body: "{}" }); toast("用户已解封"); await renderRoute(); }
catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
}
const promptScanners = [
["violent", "暴力"], ["non_violent_illegal_acts", "非暴力违法"],
["sexual_content_or_sexual_acts", "性内容"], ["pii", "个人敏感信息"],
["suicide_and_self_harm", "自杀与自残"], ["unethical_acts", "不道德行为"],
["politically_sensitive_topics", "政治敏感话题"], ["copyright_violation", "版权侵权"],
["jailbreak", "越狱攻击"],
];
async function renderPromptAuditAdmin(page) {
const params = new URLSearchParams({ page: String(promptEventPage), page_size: "20" });
Object.entries(promptEventFilters).forEach(([key, value]) => { if (value !== "" && value != null) params.set(key, value); });
const [configResult, runtimeResult, eventsResult, groupsResult] = await Promise.all([
  api("/api/admin/prompt-audit/config"), api("/api/admin/prompt-audit/runtime"),
  api(`/api/admin/prompt-audit/events?${params}`), api("/api/admin/groups"),
]);
currentPromptConfig = configResult.data;
currentPromptRuntime = runtimeResult.data;
currentPromptEndpoints = currentPromptConfig.endpoints.map(endpoint => ({ ...endpoint, token: "", clear_token: false }));
currentPromptEvents = eventsResult.data.items;
currentPromptGroups = groupsResult.data;
selectedPromptEventIds.clear();
const config = currentPromptConfig;
const runtime = currentPromptRuntime;
const events = eventsResult.data;
const selectedGroups = new Set(config.group_ids || []);
const selectedScanners = new Set(config.scanners || []);
page.innerHTML = `
  ${pageHeader("Prompt 审计", "独立 Qwen3Guard 节点池与请求前审计；完整 Prompt 不写入 SQLite", `<button class="button secondary" id="prompt-reload">重新加载</button><button class="button" id="prompt-save">保存配置</button>`)}
  <div class="metric-grid">
    ${metric("进程状态", promptProcessLabel(runtime.process_status), runtime.process_status === "degraded" ? "warn" : runtime.process_status === "running" ? "good" : "")}
    ${metric("有效模式", promptModeLabel(runtime.effective_mode))}
    ${metric("已处理", formatNumber(runtime.processed_total))}
    ${metric("阻断", formatNumber(runtime.guard_metrics.blocked), runtime.guard_metrics.blocked ? "warn" : "good")}
    ${metric("失败", formatNumber(runtime.failed_total), runtime.failed_total ? "warn" : "good")}
    ${metric("活动任务", formatNumber(runtime.queue.active))}
  </div>
  <form id="prompt-config-form" class="prompt-config">
    <section class="prompt-section prompt-mode-section">
      <div class="settings-heading"><h2>运行模式</h2><p>异步审计不影响响应；同步阻断在上游调度前完成扫描。</p></div>
      <div class="form-grid"><label class="switch-row"><span><strong>启用 Prompt 审计</strong><small>关闭后不创建任务或发送内容</small></span><input id="prompt-enabled" type="checkbox" ${config.enabled ? "checked" : ""}></label><label class="switch-row"><span><strong>同步阻断</strong><small>节点不可用时返回 503</small></span><input id="prompt-blocking" type="checkbox" ${config.blocking_enabled ? "checked" : ""}></label></div>
      <label class="switch-row compact"><span><strong>保存通过事件</strong><small>仍只保存哈希、长度和省略标记，不保存正文</small></span><input id="prompt-store-pass" type="checkbox" ${config.store_pass_events ? "checked" : ""}></label>
      <div class="prompt-runtime-strip"><span>工作槽 <strong>${formatNumber(runtime.worker_active)} / ${formatNumber(runtime.worker_total)}</strong></span><span>队列 <strong>${formatNumber(runtime.queue.active)} / ${formatNumber(runtime.queue_capacity)}</strong></span><span>每分钟完成 <strong>${formatNumber(runtime.throughput_per_minute)}</strong></span><span>平均耗时 <strong>${Number(runtime.guard_metrics.latency_avg_ms || 0).toFixed(1)} ms</strong></span><span>P50 <strong>${formatNumber(runtime.guard_metrics.latency_p50_ms)} ms</strong></span><span>P95 <strong>${formatNumber(runtime.guard_metrics.latency_p95_ms)} ms</strong></span><span>P99 <strong>${formatNumber(runtime.guard_metrics.latency_p99_ms)} ms</strong></span><span>排队 P95 <strong>${formatNumber(runtime.queue_delay_p95_ms)} ms</strong></span></div>
    </section>
    <section class="prompt-section prompt-endpoint-section">
      <div class="section-title"><div><h2>审计节点池</h2><p>按列表顺序故障切换，令牌加密保存。</p></div><button class="button secondary small" type="button" id="prompt-add-endpoint">添加节点</button></div>
      <div id="prompt-endpoint-list">${promptEndpointTable()}</div>
    </section>
    <section class="prompt-section">
      <div class="settings-heading"><h2>策略范围</h2><p>只扫描选中的路由分组和风险分类。</p></div>
      <label class="switch-row compact"><span><strong>全部路由分组</strong><small>关闭后需要至少选择一个分组</small></span><input id="prompt-all-groups" type="checkbox" ${config.all_groups ? "checked" : ""}></label>
      <div class="choice-grid prompt-group-list">${currentPromptGroups.map(group => `<label><input type="checkbox" name="prompt_group_id" value="${group.id}" ${selectedGroups.has(group.id) ? "checked" : ""}><span>${escapeHtml(group.name)}</span><small>${group.enabled ? "启用" : "停用"}</small></label>`).join("") || '<span class="field-hint">暂无路由分组</span>'}</div>
      <div class="settings-heading compact"><h2>风险分类</h2><p>节点返回的已知分类只有在此处启用后才触发对应动作。</p></div>
      <div class="check-row prompt-scanner-list">${promptScanners.map(([id, label]) => `<label><input type="checkbox" name="prompt_scanner" value="${id}" ${selectedScanners.has(id) ? "checked" : ""}> ${escapeHtml(label)}</label>`).join("")}</div>
    </section>
    <section class="prompt-section">
      <div class="settings-heading"><h2>任务容量</h2><p>Mini 不启动常驻队列进程，异步任务在当前 Tokio 运行时中执行。</p></div>
      <div class="form-grid"><div class="field"><label for="prompt-workers">工作槽</label><input id="prompt-workers" type="number" min="1" max="32" value="${config.worker_count}" required></div><div class="field"><label for="prompt-capacity">队列容量</label><input id="prompt-capacity" type="number" min="1" max="100000" value="${config.queue_capacity}" required></div></div>
      <dl class="detail-list compact-detail"><div><dt>排队 / 处理中</dt><dd>${runtime.queue.queued} / ${runtime.queue.processing}</dd></div><div><dt>完成 / 失败</dt><dd>${runtime.queue.done} / ${runtime.queue.failed}</dd></div><div><dt>排队平均 / 最大</dt><dd>${Number(runtime.queue_delay_avg_ms || 0).toFixed(1)} / ${formatNumber(runtime.queue_delay_max_ms)} ms</dd></div><div><dt>处理平均 / 最大</dt><dd>${Number(runtime.processing_avg_ms || 0).toFixed(1)} / ${formatNumber(runtime.processing_max_ms)} ms</dd></div><div><dt>最近处理</dt><dd>${formatDate(runtime.last_processed_at)}</dd></div><div><dt>最近错误</dt><dd>${escapeHtml(runtime.last_error_code || "-")}</dd></div></dl>
    </section>
    <p class="form-error" id="prompt-config-error"></p>
  </form>
  <section class="section prompt-events-section">
    <div class="section-title"><div><h2>审计事件</h2><p>${formatNumber(events.total)} 条，只展示脱敏元数据与分类证据。</p></div><div class="actions"><button class="button danger small" id="prompt-batch-delete" disabled>删除选中</button><button class="button secondary small" id="prompt-filter-delete">按范围清理</button></div></div>
    <form id="prompt-event-filter" class="filter-bar prompt-filter">
      <div class="field"><label for="prompt-decision">决策</label><select id="prompt-decision" name="decision"><option value="">全部</option><option value="pass" ${promptEventFilters.decision === "pass" ? "selected" : ""}>通过</option><option value="flag" ${promptEventFilters.decision === "flag" ? "selected" : ""}>警告</option><option value="critical" ${promptEventFilters.decision === "critical" ? "selected" : ""}>严重</option></select></div>
      <div class="field"><label for="prompt-risk">风险</label><select id="prompt-risk" name="risk_level"><option value="">全部</option>${["low","medium","high","critical"].map(value => `<option value="${value}" ${promptEventFilters.risk_level === value ? "selected" : ""}>${value}</option>`).join("")}</select></div>
      <div class="field"><label for="prompt-endpoint-filter">网关端点</label><select id="prompt-endpoint-filter" name="endpoint"><option value="">全部</option><option value="/v1/responses" ${promptEventFilters.endpoint === "/v1/responses" ? "selected" : ""}>/v1/responses</option><option value="/v1/chat/completions" ${promptEventFilters.endpoint === "/v1/chat/completions" ? "selected" : ""}>/v1/chat/completions</option></select></div>
      <div class="field"><label for="prompt-keyword">搜索</label><input id="prompt-keyword" name="keyword" value="${escapeHtml(promptEventFilters.keyword || "")}" placeholder="用户、Key、模型、分类"></div>
      <div class="field"><label for="prompt-start">开始</label><input id="prompt-start" name="start_at" type="datetime-local" value="${escapeHtml(promptEventFilters.start_at || "")}"></div>
      <div class="field"><label for="prompt-end">结束</label><input id="prompt-end" name="end_at" type="datetime-local" value="${escapeHtml(promptEventFilters.end_at || "")}"></div>
      <div class="filter-actions"><button class="button" type="submit">筛选</button><button class="button secondary" id="prompt-filter-clear" type="button">清除</button></div>
    </form>
    ${events.items.length ? promptEventTable(events.items) : emptyState("暂无 Prompt 审计事件", "启用审计并完成请求后会显示分类结果")}
    <nav class="pagination"><button class="button secondary" id="prompt-prev" ${events.page <= 1 ? "disabled" : ""}>上一页</button><span>第 ${events.page} / ${events.pages} 页</span><button class="button secondary" id="prompt-next" ${events.page >= events.pages ? "disabled" : ""}>下一页</button></nav>
  </section>`;
page.querySelector("#prompt-reload").addEventListener("click", renderRoute);
page.querySelector("#prompt-save").addEventListener("click", savePromptAudit);
page.querySelector("#prompt-add-endpoint").addEventListener("click", () => openPromptEndpoint());
bindPromptEndpointActions();
page.querySelector("#prompt-event-filter").addEventListener("submit", event => { event.preventDefault(); promptEventFilters = Object.fromEntries(new FormData(event.currentTarget)); promptEventPage = 1; renderRoute(); });
page.querySelector("#prompt-filter-clear").addEventListener("click", () => { promptEventFilters = {}; promptEventPage = 1; renderRoute(); });
page.querySelector("#prompt-prev").addEventListener("click", () => { promptEventPage -= 1; renderRoute(); });
page.querySelector("#prompt-next").addEventListener("click", () => { promptEventPage += 1; renderRoute(); });
page.querySelector("#prompt-batch-delete").addEventListener("click", openPromptBatchDelete);
page.querySelector("#prompt-filter-delete").addEventListener("click", openPromptFilterDelete);
page.querySelectorAll("[data-prompt-event]").forEach(button => button.addEventListener("click", openPromptEvent));
page.querySelectorAll("[data-prompt-select]").forEach(input => input.addEventListener("change", updatePromptSelection));
}
function promptEndpointTable() {
if (!currentPromptEndpoints.length) return emptyState("暂无审计节点", "添加至少一个 OpenAI 兼容节点后才能启用 Prompt 审计");
return `<div class="table-wrap"><table class="prompt-endpoint-table"><thead><tr><th>节点</th><th>模型</th><th>限制</th><th>凭据</th><th>状态</th><th></th></tr></thead><tbody>${currentPromptEndpoints.map(endpoint => `<tr><td><span class="cell-main">${escapeHtml(endpoint.name)}</span><span class="cell-sub mono">${escapeHtml(endpoint.base_url)}</span></td><td class="mono">${escapeHtml(endpoint.model)}</td><td>${endpoint.timeout_ms} ms<span class="cell-sub">${formatNumber(endpoint.input_limit)} 字符</span></td><td>${endpoint.token || (endpoint.has_token && !endpoint.clear_token) ? status("已配置") : status("未配置", "off")}</td><td><label class="toggle-line"><input type="checkbox" data-prompt-toggle="${escapeHtml(endpoint.id)}" ${endpoint.enabled ? "checked" : ""}> ${endpoint.enabled ? "启用" : "停用"}</label></td><td><div class="cell-actions"><button class="button quiet small" type="button" data-prompt-endpoint="probe" data-id="${escapeHtml(endpoint.id)}">探测</button><button class="button quiet small" type="button" data-prompt-endpoint="edit" data-id="${escapeHtml(endpoint.id)}">编辑</button><button class="button quiet small" type="button" data-prompt-endpoint="delete" data-id="${escapeHtml(endpoint.id)}">删除</button></div></td></tr>`).join("")}</tbody></table></div>`;
}
function refreshPromptEndpointTable() {
const container = document.querySelector("#prompt-endpoint-list"); if (!container) return;
container.innerHTML = promptEndpointTable(); bindPromptEndpointActions();
}
function bindPromptEndpointActions() {
document.querySelectorAll("[data-prompt-endpoint]").forEach(button => button.addEventListener("click", handlePromptEndpoint));
document.querySelectorAll("[data-prompt-toggle]").forEach(input => input.addEventListener("change", event => { const endpoint = currentPromptEndpoints.find(item => item.id === event.currentTarget.dataset.promptToggle); if (endpoint) endpoint.enabled = event.currentTarget.checked; refreshPromptEndpointTable(); }));
}
function openPromptEndpoint(endpoint = null) {
const item = endpoint ? { ...endpoint } : { id: `guard-${Date.now()}`, name: `Guard ${currentPromptEndpoints.length + 1}`, protocol: "openai_compatible", base_url: "http://127.0.0.1:8000", model: "sileader/qwen3guard:0.6b", timeout_ms: 3000, input_limit: 4000, enabled: true, has_token: false, token: "", clear_token: false };
openModal(endpoint ? "编辑审计节点" : "添加审计节点", `<form id="prompt-endpoint-form"><div class="form-grid"><div class="field"><label for="prompt-node-name">名称</label><input id="prompt-node-name" name="name" value="${escapeHtml(item.name)}" maxlength="128" required autofocus></div><div class="field"><label for="prompt-node-id">节点 ID</label><input id="prompt-node-id" name="id" value="${escapeHtml(item.id)}" maxlength="128" ${endpoint ? "disabled" : ""} required></div></div><div class="field"><label for="prompt-node-url">Base URL</label><input id="prompt-node-url" name="base_url" type="url" value="${escapeHtml(item.base_url)}" required></div><div class="field"><label for="prompt-node-token">API Key</label><input id="prompt-node-token" name="token" type="password" autocomplete="new-password" placeholder="${item.has_token ? "留空保留已保存令牌" : "可留空用于无认证节点"}"></div>${item.has_token ? `<label class="toggle-line"><input id="prompt-node-clear" type="checkbox" ${item.clear_token ? "checked" : ""}> 清除已保存令牌</label>` : ""}<div class="field"><label for="prompt-node-model">模型</label><input id="prompt-node-model" name="model" value="${escapeHtml(item.model)}" required></div><div class="form-grid"><div class="field"><label for="prompt-node-timeout">超时 (ms)</label><input id="prompt-node-timeout" name="timeout_ms" type="number" min="100" max="30000" value="${item.timeout_ms}" required></div><div class="field"><label for="prompt-node-limit">分块字符数</label><input id="prompt-node-limit" name="input_limit" type="number" min="128" max="100000" value="${item.input_limit}" required></div></div><label class="toggle-line"><input id="prompt-node-enabled" type="checkbox" ${item.enabled ? "checked" : ""}> 启用节点</label><p class="form-error" id="prompt-node-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-prompt-node">保存</button>`);
modal.querySelector("#save-prompt-node").addEventListener("click", () => savePromptEndpoint(endpoint?.id));
}
function savePromptEndpoint(existingId = null) {
const form = modal.querySelector("#prompt-endpoint-form"); if (!form.reportValidity()) return;
const values = Object.fromEntries(new FormData(form));
values.id = existingId || values.id.trim(); values.name = values.name.trim(); values.base_url = values.base_url.trim(); values.model = values.model.trim();
values.protocol = "openai_compatible"; values.timeout_ms = Number(values.timeout_ms); values.input_limit = Number(values.input_limit); values.enabled = form.querySelector("#prompt-node-enabled").checked; values.clear_token = form.querySelector("#prompt-node-clear")?.checked || false;
const existing = currentPromptEndpoints.find(item => item.id === values.id);
if (!existingId && existing) { form.querySelector("#prompt-node-error").textContent = "节点 ID 已存在"; return; }
const merged = { ...(existing || {}), ...values, has_token: existing?.has_token || false, token_status: existing?.token_status || "missing" };
const index = currentPromptEndpoints.findIndex(item => item.id === values.id);
if (index >= 0) currentPromptEndpoints.splice(index, 1, merged); else currentPromptEndpoints.push(merged);
closeModal(); refreshPromptEndpointTable();
}
function handlePromptEndpoint(event) {
const endpoint = currentPromptEndpoints.find(item => item.id === event.currentTarget.dataset.id); if (!endpoint) return;
const action = event.currentTarget.dataset.promptEndpoint;
if (action === "edit") return openPromptEndpoint(endpoint);
if (action === "probe") return probePromptEndpoint(endpoint, event.currentTarget);
openModal("删除审计节点", `<p>确认从配置草稿中移除 <strong>${escapeHtml(endpoint.name)}</strong>？保存配置后生效。</p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-prompt-node-delete">移除</button>`);
modal.querySelector("#confirm-prompt-node-delete").addEventListener("click", () => { currentPromptEndpoints = currentPromptEndpoints.filter(item => item.id !== endpoint.id); closeModal(); refreshPromptEndpointTable(); });
}
async function probePromptEndpoint(endpoint, button) {
button.disabled = true;
try { const result = await api("/api/admin/prompt-audit/endpoints/probe", { method: "POST", body: JSON.stringify({ endpoint }) }); toast(`${endpoint.name}: ${result.data.message}${result.data.http_status ? ` (${result.data.http_status})` : ""}`, !result.data.ok); }
catch (error) { toast(error.message, true); }
finally { button.disabled = false; }
}
async function savePromptAudit() {
const form = document.querySelector("#prompt-config-form"); if (!form.reportValidity()) return;
const button = document.querySelector("#prompt-save"); const error = form.querySelector("#prompt-config-error");
const payload = { expected_config_version: currentPromptConfig.config_version, enabled: form.querySelector("#prompt-enabled").checked, blocking_enabled: form.querySelector("#prompt-blocking").checked, store_pass_events: form.querySelector("#prompt-store-pass").checked, strategy: "priority", worker_count: Number(form.querySelector("#prompt-workers").value), queue_capacity: Number(form.querySelector("#prompt-capacity").value), scanners: [...form.querySelectorAll('[name="prompt_scanner"]:checked')].map(input => input.value), all_groups: form.querySelector("#prompt-all-groups").checked, group_ids: [...form.querySelectorAll('[name="prompt_group_id"]:checked')].map(input => Number(input.value)), endpoints: currentPromptEndpoints.map(endpoint => ({ id:endpoint.id, name:endpoint.name, protocol:"openai_compatible", base_url:endpoint.base_url, model:endpoint.model, token:endpoint.token || undefined, clear_token:Boolean(endpoint.clear_token), timeout_ms:Number(endpoint.timeout_ms), input_limit:Number(endpoint.input_limit), enabled:Boolean(endpoint.enabled) })) };
button.disabled = true; error.textContent = "";
try { await api("/api/admin/prompt-audit/config", { method: "PUT", body: JSON.stringify(payload) }); toast("Prompt 审计配置已保存"); await renderRoute(); }
catch (requestError) { error.textContent = requestError.message; }
finally { button.disabled = false; }
}
function promptEventTable(rows) {
return `<div class="table-wrap"><table class="prompt-event-table"><thead><tr><th></th><th>时间</th><th>用户 / Key</th><th>端点 / 模型</th><th>决策</th><th>分类</th><th>耗时</th><th></th></tr></thead><tbody>${rows.map(row => `<tr><td><input type="checkbox" data-prompt-select="${row.id}" ${selectedPromptEventIds.has(row.id) ? "checked" : ""} aria-label="选择事件 ${row.id}"></td><td>${formatDate(row.created_at)}</td><td><span class="cell-main">${escapeHtml(row.snapshot.username || "-")}</span><span class="cell-sub">${escapeHtml(row.snapshot.api_key_name || "-")}</span></td><td><span class="cell-main mono">${escapeHtml(row.snapshot.endpoint)}</span><span class="cell-sub mono">${escapeHtml(row.snapshot.model || "-")}</span></td><td>${promptDecisionStatus(row)}</td><td>${row.categories.length ? row.categories.map(item => `<span class="status ${row.action === "Block" ? "error" : "warn"}">${escapeHtml(item)}</span>`).join(" ") : "-"}</td><td>${row.latency_ms} ms</td><td><button class="button quiet small" data-prompt-event="${row.id}">详情</button></td></tr>`).join("")}</tbody></table></div>`;
}
function updatePromptSelection(event) {
const id = Number(event.currentTarget.dataset.promptSelect); if (event.currentTarget.checked) selectedPromptEventIds.add(id); else selectedPromptEventIds.delete(id);
const button = document.querySelector("#prompt-batch-delete"); button.disabled = selectedPromptEventIds.size === 0; button.textContent = selectedPromptEventIds.size ? `删除选中 (${selectedPromptEventIds.size})` : "删除选中";
}
function openPromptEvent(event) {
const item = currentPromptEvents.find(row => String(row.id) === String(event.currentTarget.dataset.promptEvent)); if (!item) return;
const issues = item.issue_summaries || [];
openModal("Prompt 审计详情", `<dl class="detail-list"><div><dt>请求 ID</dt><dd class="mono">${escapeHtml(item.snapshot.request_id)}</dd></div><div><dt>用户 / API Key</dt><dd>${escapeHtml(item.snapshot.username || "-")} / ${escapeHtml(item.snapshot.api_key_name || "-")}</dd></div><div><dt>端点 / 模型</dt><dd class="mono">${escapeHtml(item.snapshot.endpoint)} / ${escapeHtml(item.snapshot.model || "-")}</dd></div><div><dt>Prompt SHA-256</dt><dd class="mono">${escapeHtml(item.snapshot.prompt_hash)}</dd></div><div><dt>内容保留</dt><dd>${escapeHtml(item.snapshot.redacted_preview)}；完整正文未持久化</dd></div><div><dt>长度 / 消息数</dt><dd>${item.snapshot.prompt_length} 字符 / ${item.snapshot.message_count}</dd></div><div><dt>决策</dt><dd>${escapeHtml(item.decision)} / ${escapeHtml(item.risk_level)} / ${escapeHtml(item.action)}</dd></div><div><dt>Guard 节点</dt><dd>${escapeHtml(item.guard_endpoint_id || "-")}</dd></div><div><dt>配置 / 分块</dt><dd>v${item.config_version} / ${item.chunk_total}</dd></div><div><dt>耗时</dt><dd>${item.latency_ms} ms</dd></div></dl>${issues.length ? `<div class="prompt-issue-list">${issues.map(issue => `<article><header><strong>${escapeHtml(issue.title)}</strong>${status(issue.severity, item.action === "Block" ? "error" : "warn")}</header><p>${escapeHtml(issue.description)}</p><code>${escapeHtml(issue.code)}</code><span>score ${Number(issue.score).toFixed(2)}</span></article>`).join("")}</div>` : ""}<p class="form-error" id="prompt-event-error"></p>`, `<button class="button secondary" data-close-modal>关闭</button><button class="button danger" id="delete-prompt-event">删除事件</button>`);
modal.querySelector("#delete-prompt-event").addEventListener("click", async click => { click.currentTarget.disabled = true; try { await api(`/api/admin/prompt-audit/events/${item.id}`, { method: "DELETE" }); closeModal(); toast("Prompt 审计事件已删除"); await renderRoute(); } catch (error) { modal.querySelector("#prompt-event-error").textContent = error.message; click.currentTarget.disabled = false; } });
}
function openPromptBatchDelete() {
const ids = [...selectedPromptEventIds]; if (!ids.length) return;
openModal("删除选中事件", `<p>将删除 ${ids.length} 条事件以及不再被引用的任务记录。</p><p class="form-error" id="prompt-delete-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-prompt-batch-delete">删除</button>`);
modal.querySelector("#confirm-prompt-batch-delete").addEventListener("click", async click => { click.currentTarget.disabled = true; try { const result = await api("/api/admin/prompt-audit/events/batch-delete", { method: "POST", body: JSON.stringify({ ids }) }); closeModal(); toast(`已删除 ${result.data.deleted_events} 条事件`); await renderRoute(); } catch (error) { modal.querySelector("#prompt-delete-error").textContent = error.message; click.currentTarget.disabled = false; } });
}
function openPromptFilterDelete() {
openModal("按时间范围清理", `<form id="prompt-delete-range-form"><div class="field"><label for="prompt-delete-preset">范围</label><select id="prompt-delete-preset"><option value="7">删除 7 天前事件</option><option value="30" selected>删除 30 天前事件</option><option value="90">删除 90 天前事件</option><option value="all">删除全部匹配事件</option><option value="custom">自定义时间</option></select></div><div class="form-grid"><div class="field"><label for="prompt-delete-start">开始</label><input id="prompt-delete-start" type="datetime-local"></div><div class="field"><label for="prompt-delete-end">结束</label><input id="prompt-delete-end" type="datetime-local"></div></div><p class="field-hint">预览使用事件 ID 高水位，确认期间新产生的事件不会被删除。</p><p class="form-error" id="prompt-delete-range-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="preview-prompt-filter-delete">预览</button>`);
const form = modal.querySelector("#prompt-delete-range-form");
const resolveRange = () => { const preset = form.querySelector("#prompt-delete-preset").value; if (preset === "custom") return; const now = Date.now(); form.querySelector("#prompt-delete-start").value = toDateTimeLocal(new Date(0).toISOString()); form.querySelector("#prompt-delete-end").value = toDateTimeLocal(new Date(preset === "all" ? now : now - Number(preset) * 86400000).toISOString()); };
form.querySelector("#prompt-delete-preset").addEventListener("change", resolveRange); resolveRange();
modal.querySelector("#preview-prompt-filter-delete").addEventListener("click", previewPromptFilterDelete);
}
async function previewPromptFilterDelete(event) {
const form = modal.querySelector("#prompt-delete-range-form"); const error = form.querySelector("#prompt-delete-range-error");
const start = form.querySelector("#prompt-delete-start").value; const end = form.querySelector("#prompt-delete-end").value;
if (!start || !end || new Date(start) >= new Date(end)) { error.textContent = "请选择有效的开始和结束时间"; return; }
const filter = { ...promptEventFilters, start_at: new Date(start).toISOString(), end_at: new Date(end).toISOString() };
event.currentTarget.disabled = true;
try {
  const preview = await api("/api/admin/prompt-audit/events/delete-preview", { method: "POST", body: JSON.stringify(filter) });
  openModal("确认范围清理", `<p>筛选范围匹配 <strong>${preview.data.matched_count}</strong> 条事件。</p><dl class="detail-list"><div><dt>高水位 ID</dt><dd>${preview.data.snapshot_max_id}</dd></div><div><dt>确认有效期</dt><dd>${formatDate(preview.data.expires_at)}</dd></div><div><dt>筛选摘要</dt><dd class="mono">${escapeHtml(JSON.stringify(preview.data.filter_summary))}</dd></div></dl><p class="form-error" id="prompt-filter-delete-error"></p>`, `<button class="button secondary" data-close-modal>取消</button><button class="button danger" id="confirm-prompt-filter-delete" ${preview.data.matched_count ? "" : "disabled"}>确认删除</button>`);
  modal.querySelector("#confirm-prompt-filter-delete")?.addEventListener("click", async click => { click.currentTarget.disabled = true; try { const result = await api("/api/admin/prompt-audit/events/delete-by-filter", { method: "POST", body: JSON.stringify({ filter, snapshot_max_id: preview.data.snapshot_max_id, filter_hash: preview.data.filter_hash, confirmation_token: preview.data.confirmation_token, confirm: true }) }); closeModal(); toast(`已删除 ${result.data.deleted_events} 条事件`); await renderRoute(); } catch (deleteError) { modal.querySelector("#prompt-filter-delete-error").textContent = deleteError.message; click.currentTarget.disabled = false; } });
} catch (requestError) { error.textContent = requestError.message; event.currentTarget.disabled = false; }
}
function promptDecisionStatus(row) { return row.action === "Block" ? status("阻断", "error") : row.action === "Warn" ? status("警告", "warn") : status("通过"); }
function promptModeLabel(mode) { return ({ off:"关闭", async_audit:"异步审计", blocking:"同步阻断" })[mode] || mode; }
function promptProcessLabel(value) { return ({ disabled:"停用", running:"运行中", degraded:"降级", error:"错误" })[value] || value; }
function openModal(title, body, footer = "") {
modal.className = "modal";
modal.innerHTML = `<div class="modal-header"><h2>${escapeHtml(title)}</h2><button class="modal-close" data-close-modal aria-label="关闭">&times;</button></div><div class="modal-body">${body}</div>${footer ? `<div class="modal-footer">${footer}</div>` : ""}`;
modal.querySelectorAll("[data-close-modal]").forEach(button => button.addEventListener("click", closeModal));
if (!modal.open) modal.showModal();
}
function closeModal() { modal.close(); }
function pageHeader(title, subtitle, actions = "") {
return `<header class="page-header"><div><h1>${escapeHtml(title)}</h1><p>${escapeHtml(subtitle)}</p></div>${actions ? `<div class="actions">${actions}</div>` : ""}</header>`;
}
function metric(label, value, tone = "") { return `<article class="metric ${tone}"><span>${escapeHtml(label)}</span><strong>${escapeHtml(String(value))}</strong></article>`; }
function status(label, tone = "") { return `<span class="status ${tone}">${escapeHtml(label)}</span>`; }
function emptyState(title, text, buttonText = "", buttonId = "") { return `<section class="empty"><h2>${escapeHtml(title)}</h2><p>${escapeHtml(text)}</p>${buttonText ? `<button class="button" id="${buttonId}">${escapeHtml(buttonText)}</button>` : ""}</section>`; }
function formatNumber(value) { return new Intl.NumberFormat("zh-CN", { notation: Number(value) > 9999 ? "compact" : "standard" }).format(Number(value) || 0); }
function formatMoney(cents) { return new Intl.NumberFormat("zh-CN", { style: "currency", currency: "CNY" }).format((Number(cents) || 0) / 100); }
function formatUsdMicros(value) { return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 }).format((Number(value) || 0) / 1000000); }
function formatDate(value) { if (!value) return "-"; const parsed = new Date(value.includes("T") ? value : `${value.replace(" ", "T")}Z`); return Number.isNaN(parsed.valueOf()) ? value : new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(parsed); }
function toDateTimeLocal(value) { if (!value) return ""; const date = new Date(value); if (Number.isNaN(date.valueOf())) return ""; const local = new Date(date.getTime() - date.getTimezoneOffset() * 60000); return local.toISOString().slice(0, 16); }
function parseModelList(value) { return String(value || "").split(/[\n,]/).map(item => item.trim()).filter(Boolean); }
function escapeHtml(value) { return String(value ?? "").replace(/[&<>'"]/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[char])); }
function toast(message, error = false) { const item = document.createElement("div"); item.className = `toast${error ? " error" : ""}`; item.textContent = message; toastRegion.append(item); setTimeout(() => item.remove(), 3600); }
