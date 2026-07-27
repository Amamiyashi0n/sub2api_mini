# Sub2API page migration status

This file tracks functional migration from `../sub2api/frontend`. A route is only
marked complete when its user-visible operations and supporting API behavior are
available in Sub2API Mini. Visual parity with the Vue application is not required.

Status values:

- `complete`: the Mini page covers the original page's applicable behavior.
- `partial`: a usable subset exists, with missing behavior listed below.
- `pending`: no Mini equivalent exists yet.
- `not-applicable`: intentionally excluded only after an explicit product decision.

## Public and authentication

| Original route | Status | Mini route | Remaining behavior |
| --- | --- | --- | --- |
| `/setup` | complete | `#/setup` | - |
| `/home` | not-applicable | - | Public homepage explicitly removed |
| `/login` | complete | `#/overview` | - |
| `/register` | complete | `#/register` | - |
| `/email-verify` | complete | `#/email-verify` | - |
| `/forgot-password` | complete | `#/forgot-password` | - |
| `/reset-password` | complete | `#/reset-password` | - |
| `/key-usage` | not-applicable | - | Unauthenticated API Key lookup explicitly removed |
| `/legal/:documentId` | partial | `#/page/:slug` | Login is required |

Third-party user login providers are intentionally excluded. The localhost callback
on port `1455` is retained only for upstream OpenAI/Codex account authorization.

## User console

| Original route | Status | Mini route | Remaining behavior |
| --- | --- | --- | --- |
| `/dashboard` | complete | `#/overview` | - |
| `/keys` | complete | `#/keys` | - |
| `/batch-image` | complete | `#/batchImages` | - |
| `/usage` | complete | `#/usage` | - |
| `/redeem` | complete | `#/redeem` | - |
| `/available-channels` | complete | `#/models`, `#/channels` | - |
| `/profile` | complete | `#/profile` | - |
| `/subscriptions` | complete | `#/subscriptions` | - |
| `/custom/:id` | complete | `#/pages`, `#/page/:slug` | - |
| `/monitor` | complete | `#/monitor` | - |

## Administrator console

| Original route | Status | Mini route | Remaining behavior |
| --- | --- | --- | --- |
| `/admin/dashboard` | complete | `#/overview` | - |
| `/admin/ops` | complete | `#/opsAdmin` | Same-origin SSE replaces WebSocket for lower idle memory; minute rollups, exact stream-lifetime concurrency, TTFT/retry/switch telemetry, scheduled digest reports, runtime log-sink controls, request details, alerting and process/SQLite metrics are implemented |
| `/admin/audit-logs` | complete | `#/audit` | - |
| `/admin/users` | complete | `#/users` | - |
| `/admin/groups` | complete | `#/groups` | - |
| `/admin/channels/pricing` | complete | `#/channelsAdmin` | - |
| `/admin/channels/monitor` | complete | `#/monitorAdmin` | - |
| `/admin/subscriptions` | complete | `#/plans` | - |
| `/admin/accounts` | complete | `#/accounts` | OpenAI API Key, OAuth and linked Spark shadow accounts; browser-PKCE/auth.json re-authentication, scheduled tests, statistics, credential-safe duplication, backup import/export, transactional bulk management, proxy/group binding, connectivity tests and runtime recovery are implemented |
| `/admin/announcements` | complete | `#/content` | - |
| `/admin/proxies` | complete | `#/proxies` | - |
| `/admin/redeem` | complete | `#/redeemAdmin` | - |
| `/admin/settings` | complete | `#/settings` | - |
| `/admin/risk-control` | complete | `#/riskAdmin` | Encrypted moderation key pool with persistent load/health/freeze statistics, least-load scheduling and retries; observe/pre-block modes, sampling, group/model scope, thresholds, local keywords, known-hash precheck, automatic user bans, user email notifications, retention, key tests and filtered logs are implemented |
| `/admin/prompt-audit` | complete | `#/promptAuditAdmin` | Dynamic worker-slot concurrency, persistent queue/processing timings, true P50/P95/P99 metrics, throughput, restart recovery, encrypted endpoint pools, optimistic config versions, async/blocking modes, strict Qwen3Guard parsing, scanner/group policies, chunking/failover, probes, redacted events, detail, filters and safe deletion are implemented; raw-prompt persistence remains intentionally prohibited by the no-request-body policy |
| `/admin/usage` | complete | `#/usage` | - |
| `/admin/orders/dashboard` | complete | `#/ordersAdmin` | - |
| `/admin/orders` | complete | `#/ordersAdmin` | - |
| `/admin/orders/plans` | complete | `#/plans` | - |

## Mini-only runtime pages

| Mini route | Purpose |
| --- | --- |
| `#/status` | Single-process health, version and endpoint information |

## Current migration batch

- Added authenticated profile reads and display-name updates.
- Added current-password verification and password changes that revoke other sessions.
- Removed the unauthenticated public homepage and API-key usage lookup, including
  their backend routes, embedded pages and navigation links.
- Added API-key expiry, token quota and model allow-list policies, enforced before
  forwarding requests and reflected in `/v1/models`.
- Added an authenticated model catalog aggregated from enabled upstream accounts
  with the existing five-minute cache.
- Added user/admin usage filtering, paging and request detail views with user
  ownership checks.
- Added announcement drafting/publication, display windows, notify modes and
  per-user read state.
- Added managed legal/custom text pages with authenticated visibility.
- Added total usage, average latency, seven-day trend and top-model aggregates to
  administrator and user dashboards.
- Added live retry/cache/cooldown runtime settings, login branding and audit
  retention controls.
- Added administrator mutation audit metadata without recording request bodies or
  secrets, plus filters and detail inspection.
- Added upstream account editing and manual cooldown recovery.
- Added routing-group CRUD, account membership, Key assignment, group-aware
  scheduling and group-level model allow lists.
- Added Token plan CRUD, administrator subscription assignment/cancellation,
  user progress/history and gateway enforcement for active subscription quotas.
- Added redeem-code generation and management, one-time plaintext display, hashed
  code storage, per-user redemption history and atomic subscription activation.
- Added email identities and login, registration switches, hashed one-time email
  verification/reset challenges and session revocation after password reset.
- Added embedded registration, password recovery and reset pages.
- Added optional HTTPS mail-webhook delivery without a resident mail worker or an
  additional runtime process.
- Added balance-funded plan purchases, user order history, administrator order
  metrics and atomic refunds that restore balance and cancel linked subscriptions.
- Added encrypted HTTP/HTTPS/SOCKS proxy management, expiry and connectivity
  testing, account assignment, proxy-aware OAuth/gateway requests and strict
  no-direct-fallback scheduling for unavailable proxies.
- Added proxy exit metadata, parallel OpenAI quality checks, search/filter and
  batch actions, portable encrypted-data import/export, usage detail, and
  cycle-safe backup-proxy or explicit direct fallback policies.
- Added encrypted channel monitor definitions, provider-aware manual and scheduled
  probes, persistent history, 7/15/30-day availability, global runtime controls,
  and embedded administrator/user status views.
- Added encrypted TOTP secrets, password-gated setup, two-stage login challenges,
  one-time recovery codes, profile controls, and TOTP-protected audit-log cleanup.
- Added channel CRUD and group binding, model-restriction enforcement, token and
  per-request prices stored as integer microusd, gateway cost calculation, and
  administrator/user pricing views.
- Added low-memory in-process risk control with encrypted moderation API keys,
  local keyword and known-hash checks, observe/pre-block modes, group/model scope,
  configurable category thresholds, automatic user bans, retention, filtered
  logs and an embedded administrator console. Request text is never persisted.
- Enforced disabled-user ownership during downstream API-key authentication so
  administrative and automatic bans immediately revoke gateway access.
- Added in-process Prompt Audit with encrypted OpenAI-compatible Guard endpoints,
  strict Qwen3Guard parsing, async and blocking modes, scanner/group scope,
  chunking and endpoint failover, task/event persistence, runtime and probe views,
  plus single, batch and high-watermark-confirmed filtered deletion. Prompt bodies
  are sent only to configured Guard nodes and are never persisted in SQLite.
- Added complete downstream Key policies with exact integer-microusd total and
  rolling 5-hour/1-day/7-day cost limits, reset watermarks, custom one-time-display
  keys, model/group restrictions, expiry, CIDR allow/deny lists, direct peer-IP
  enforcement, last-used IP, filters and ownership-safe batch actions.
- Added complete administrator user operations with searchable aggregate usage,
  rich details for Keys/subscriptions/orders/trends, editable identity and notes,
  atomic balance adjustments with immutable history, password/session revocation,
  administrator-safe batch actions and history-preserving soft deletion. The
  embedded user-management page is loaded on demand from a fixed asset route.
- Added a low-memory operations console with six time ranges, polling QPS/TPS,
  latency percentiles and histograms, model/error distributions, account and
  process health, filtered request details, merged audit/error logs, persistent
  alert rules and events, automatic resolution, optional webhook email delivery,
  and on-demand embedded frontend loading without request-body capture.
- Added complete user and administrator usage analytics with cache/reasoning Token
  capture for JSON and SSE responses, request type and service-tier dimensions,
  aggregate trends, model/error breakdowns, filters and streaming CSV export.
- Added five-minute, one-time administrator cleanup previews bound to canonical
  filters and a snapshot high-watermark so requests created after preview cannot
  be deleted by confirmation.
- Removed public API-key usage; equivalent owner-scoped usage remains available
  after login through the user console.
- Completed user dashboards with balance, Key counts, active subscription progress,
  RPM/TPM, cache/reasoning Token, cost, range switching, request charts, model and
  endpoint distributions, quick actions, announcements and recent usage.
- Completed administrator dashboards with user growth, account/Key health, success
  and Token metrics, order revenue/refunds, active subscriptions, request/cost
  charts, top-user spending and routing-group distributions.
- Added the fixed `/setup/status` compatibility contract and a public deployment
  readiness page covering SQLite connectivity, WAL, foreign keys, migrations,
  persistent storage, administrator bootstrap, listeners and single-process mode.
  PostgreSQL and Redis setup steps are replaced by the confirmed SQLite/no-Redis
  architecture, while secret environment values remain server-only.
- Added password-gated profile email binding, replacement and removal with
  user-scoped hashed challenges, verified-email uniqueness, one-time confirmation
  and revocation of other sessions.
- Added the dedicated `#/email-verify` registration step with retry countdown and
  session-scoped pending registration data, while retaining atomic challenge
  consumption and account creation.
- Added encrypted Gemini batch-image providers with model policies, priorities,
  concurrency limits, connectivity probes and administrator controls.
- Added persistent asynchronous batch-image jobs with per-Key model enforcement,
  idempotent submission, atomic balance holds, success-count settlement, polling,
  cancellation, crash recovery and automatic output retention cleanup.
- Added reference images, safe local output indexing, single-image and ZIP
  downloads, record hiding, output deletion and failed-item retry ancestry without
  persisting Prompt bodies in SQLite or logs.
- Added the lazy embedded `#/batchImages` user and administrator console while
  keeping the primary application script below 256 KiB.
- Added Vertex batch-image providers with validated encrypted service-account
  credentials, RS256 OAuth token exchange and bounded access-token caching.
- Added managed GCS JSONL upload, Vertex batch submission/polling/cancellation,
  result-object aggregation and ownership-checked input/output cleanup.
- Snapshotted each job's non-secret provider routing configuration so provider
  edits cannot redirect an active batch, while retaining encrypted credentials
  required for historical jobs to finish.
- Added safe server-side Markdown rendering with tables, code blocks and generated
  page tables of contents; raw HTML and active URL schemes are never emitted.
- Completed custom content iframe mode for bounded credential-free HTTP(S) URLs,
  including private-page reads through authenticated user APIs.
- Retained configurable login branding while removing contact, docs and
  Markdown-or-iframe public-home configuration.
- Completed the user monitor view with persisted 30/60/120-second auto-refresh,
  a live countdown and compact 24-result status timelines on desktop and mobile.
- Completed announcement targeting with OR/AND balance and active-plan rules,
  authenticated-content isolation, visibility-safe read marking, unread-first user views,
  automatic popup notices and searchable eligible/read status for every user.
- Added routing-group platform and standard/subscription metadata, exact
  millionth-unit default and per-user rate multipliers, server-timezone peak
  windows, and token-only gateway cost multiplication with deterministic integer
  arithmetic.
- Completed the available-channel view with platform sections, normalized
  platform metadata, model pricing, searchable channel/group/model content,
  default/user/effective multiplier badges and explicit peak-window timezone.
- Added transactional channel Token intervals with original `(min,max]` matching,
  partial-price fallback, overlap validation and exact integer microusd storage.
- Added flat and per-interval cache-read/cache-write configuration, cache-hit
  extraction-aware billing that excludes cached Token from ordinary input cost,
  and interval/cache details in administrator and user channel views.
- Added original-compatible `sub2api-data`/`sub2api-bundle` v1 account backup
  import/export with multi-file UI, proxy reuse and fallback restoration,
  per-item results, OpenAI API Key/Codex OAuth support and encrypted-at-rest
  credential verification. Unsupported original platforms fail per item without
  rolling back valid entries.
- Added the original three-step CRS sync workflow with preview selection,
  existing-account merge updates, optional proxy synchronization and per-item
  results for Claude OAuth, Claude Setup Token, Claude Console API Key, OpenAI
  OAuth and OpenAI Responses API Key accounts.
- Added account multi-selection with transactional bulk scheduling, priority,
  concurrency, proxy and group updates, runtime-state recovery, deletion and
  per-item OAuth refresh results. Manual OAuth refresh now bypasses the normal
  pre-expiry short circuit while automatic gateway refresh keeps it.
- Added 1/7/30/90-day per-account usage statistics with daily trends, success,
  Token, cost, model and endpoint aggregates. Added paused account duplication
  with fresh credential encryption and copied group bindings, plus auth.json
  OAuth re-authentication that preserves scheduling configuration.
- Added five-field Cron account test plans with bounded concurrent execution,
  atomic due-plan claims, retained success/error history, manual runs and
  successful-test account recovery, plus a lazy per-account management panel.
- Added browser PKCE re-authentication bound to an existing OAuth account with
  one-time state, credential replacement, missing-field preservation and a
  distinct callback result, while retaining auth.json as an offline path.
- Added one-per-parent OpenAI Spark shadow accounts with independent scheduling,
  groups, usage and test plans but live inherited OAuth credentials and proxy;
  shadow credentials cannot diverge and backup export skips linked shadows.
- Completed public, exclusive and subscription-group authorization with
  administrator-managed user grants, plan-to-group binding, concurrent
  subscriptions across groups, per-group Token accounting and immediate gateway
  revocation when a grant or subscription is no longer active.
- Added persistent balance auto-renewal with per-period idempotency, atomic debit,
  renewal orders, insufficient-balance and unavailable-plan retry
  state, user controls, administrator visibility and cancellation-safe cleanup.
- Added plan original-price, ISO display-currency, product-name and structured
  feature fields across administrator editing and user cards.
- Added encrypted SQLite SMTP settings with Webhook/SMTP/automatic transport
  selection, STARTTLS and implicit TLS, loopback-only plaintext mode, connection
  and test-email controls, bounded sessions and unified auth/risk/ops delivery.
- Completed low-memory operations telemetry with five-minute-safe minute rollups,
  same-origin SSE live QPS/TPS and active-stream metrics, TTFT/retry/account-switch
  request details, bounded redacted runtime logs, dynamic log controls, retention,
  alert evaluation and daily/weekly/manual email reports.
- Added channel model mapping with exact and longest-prefix wildcard rules,
  requested/mapped billing-model selection and preserved usage lineage. Cache
  writes and image input/output Token now use their configured flat or interval
  prices, with the advanced channel editor loaded on demand.
- Completed account-stat pricing with ordered account/group scopes, dedicated
  model and interval prices, upstream-model matching, separate account/user
  costs, and optional reuse of channel prices before group multipliers.
- Completed encrypted channel-monitor request templates with explicit snapshot
  application, association-safe deletion, and separate origin-Ping and business
  request latency across administrator history and public status views.
