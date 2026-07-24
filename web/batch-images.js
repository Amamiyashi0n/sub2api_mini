"use strict";

window.Sub2MiniBatchImages = (() => {
  const terminal = new Set(["completed", "failed", "cancelled", "output_deleted"]);
  let data = { keys: [], models_by_key: [], jobs: [], providers: [], has_more: false, next_cursor: null };
  let filters = { status: "", api_key_id: "", task_name: "", downloaded: "" };
  let pollTimer = null;
  let itemSequence = 0;

  function time(value) {
    if (!value) return "-";
    const date = new Date(typeof value === "number" ? value * 1000 : value);
    return Number.isNaN(date.valueOf()) ? "-" : new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(date);
  }

  function jobStatus(value) {
    const labels = { created: "创建中", queued: "已排队", running: "生成中", indexing: "整理输出", settling: "结算中", completed: "已完成", failed: "失败", cancelled: "已取消", output_deleted: "输出已清理" };
    const tone = value === "completed" ? "" : value === "failed" ? "error" : value === "cancelled" || value === "output_deleted" ? "off" : "warn";
    return status(labels[value] || value, tone);
  }

  function itemStatus(value) {
    const labels = { pending: "等待", succeeded: "成功", failed: "失败", cancelled: "取消" };
    return status(labels[value] || value, value === "failed" ? "error" : value === "pending" ? "warn" : value === "cancelled" ? "off" : "");
  }

  function enabledKeys() { return data.keys.filter(key => key.enabled); }
  function modelsForKey(keyId) {
    const row = data.models_by_key.find(item => Number(item.api_key_id) === Number(keyId));
    const seen = new Set();
    return (row?.models || []).filter(model => !seen.has(model.id) && seen.add(model.id));
  }

  function metrics() {
    const active = data.jobs.filter(job => !terminal.has(job.status)).length;
    const completed = data.jobs.filter(job => job.status === "completed").length;
    return `${metric("可用余额", formatMoney(data.balance_cents), "good")}${metric("冻结余额", formatMoney(data.frozen_balance_cents), data.frozen_balance_cents ? "warn" : "")}${metric("活动任务", active, active ? "warn" : "good")}${metric("本页完成", completed)}`;
  }

  function jobTable(rows) {
    if (!rows.length) return emptyState("暂无批任务", "当前筛选条件没有任务", "创建任务", "empty-create-batch");
    return `<div class="table-wrap"><table class="batch-job-table"><thead><tr><th>任务</th><th>状态</th><th>模型</th><th>结果</th><th>金额</th><th class="hide-mobile">创建时间</th><th></th></tr></thead><tbody>${rows.map(job => `<tr>
      <td><span class="cell-main">${escapeHtml(job.task_name)}</span><span class="cell-sub mono">${escapeHtml(job.id)}</span></td>
      <td>${jobStatus(job.status)}${job.error ? `<span class="cell-sub text-danger">${escapeHtml(job.error.message)}</span>` : ""}</td>
      <td><span class="cell-main mono">${escapeHtml(job.model)}</span><span class="cell-sub">${escapeHtml(job.provider_name)}</span></td>
      <td><span class="cell-main">${job.generated_image_count} / ${job.requested_image_count}</span><span class="cell-sub">${job.success_count} 成功 · ${job.fail_count} 失败</span></td>
      <td><span class="cell-main">${job.actual_cost_cents == null ? formatMoney(job.estimated_cost_cents) : formatMoney(job.actual_cost_cents)}</span><span class="cell-sub">冻结 ${formatMoney(job.hold_amount_cents)}</span></td>
      <td class="hide-mobile">${time(job.created_at)}</td>
      <td><div class="cell-actions"><button class="button quiet small" data-batch-action="detail" data-id="${escapeHtml(job.id)}">详情</button>${job.status === "completed" && job.success_count ? `<button class="button quiet small" data-batch-action="download" data-id="${escapeHtml(job.id)}">ZIP</button>` : ""}${!terminal.has(job.status) ? `<button class="button quiet small" data-batch-action="cancel" data-id="${escapeHtml(job.id)}">取消</button>` : ""}</div></td>
    </tr>`).join("")}</tbody></table></div>`;
  }

  function providerPanel() {
    if (state.role !== "admin") return "";
    return `<section class="section batch-provider-section"><div class="section-title"><h2>图片提供商</h2><button class="button secondary small" id="add-batch-provider">添加</button></div>
      ${data.providers.length ? `<div class="batch-provider-grid">${data.providers.map(provider => `<article class="batch-provider-card"><header><div><h3>${escapeHtml(provider.name)}</h3><code>${escapeHtml(provider.kind === "vertex" ? `${provider.project_id} · ${provider.location} · gs://${provider.gcs_bucket}` : provider.base_url)}</code></div>${provider.enabled ? status(provider.kind === "vertex" ? "Vertex" : "Gemini") : status("停用", "off")}</header><div class="batch-provider-stats"><span>优先级 <strong>${provider.priority}</strong></span><span>并发 <strong>${provider.concurrency}</strong></span><span>单价 <strong>${formatMoney(provider.unit_price_cents)}</strong></span><span>批折扣 <strong>${(provider.batch_discount_bps / 100).toFixed(0)}%</strong></span></div><div class="model-cloud compact">${provider.models.map(model => `<code>${escapeHtml(model)}</code>`).join("")}</div>${provider.last_error ? `<p class="batch-provider-error">${escapeHtml(provider.last_error)}</p>` : ""}<footer><button class="button quiet small" data-provider-action="test" data-id="${provider.id}">探测</button><button class="button quiet small" data-provider-action="toggle" data-id="${provider.id}">${provider.enabled ? "停用" : "启用"}</button><button class="button quiet small" data-provider-action="edit" data-id="${provider.id}">编辑</button><button class="button quiet small" data-provider-action="delete" data-id="${provider.id}">删除</button></footer></article>`).join("")}</div>` : emptyState("暂无图片提供商", "添加 Gemini API 或 Vertex 后可提交批任务", "添加提供商", "empty-add-provider")}</section>`;
  }

  function filtersMarkup() {
    return `<form class="batch-filter" id="batch-filter-form"><div class="field"><label for="batch-task-filter">任务名</label><input id="batch-task-filter" name="task_name" type="search" value="${escapeHtml(filters.task_name)}"></div><div class="field"><label for="batch-status-filter">状态</label><select id="batch-status-filter" name="status"><option value="">全部</option>${["queued", "running", "completed", "failed", "cancelled", "output_deleted"].map(value => `<option value="${value}" ${filters.status === value ? "selected" : ""}>${({ queued:"排队", running:"运行中", completed:"完成", failed:"失败", cancelled:"取消", output_deleted:"输出已清理" })[value]}</option>`).join("")}</select></div><div class="field"><label for="batch-key-filter">API Key</label><select id="batch-key-filter" name="api_key_id"><option value="">全部</option>${data.keys.map(key => `<option value="${key.id}" ${String(filters.api_key_id) === String(key.id) ? "selected" : ""}>${escapeHtml(key.name)}</option>`).join("")}</select></div><div class="field"><label for="batch-download-filter">下载</label><select id="batch-download-filter" name="downloaded"><option value="">全部</option><option value="true" ${filters.downloaded === "true" ? "selected" : ""}>已下载</option><option value="false" ${filters.downloaded === "false" ? "selected" : ""}>未下载</option></select></div><div class="batch-filter-actions"><button class="button secondary" type="submit">筛选</button><button class="button quiet" type="button" id="clear-batch-filter">清除</button></div></form>`;
  }

  async function render(page) {
    clearTimeout(pollTimer);
    const requests = [api("/api/user/batch-images/bootstrap")];
    if (state.role === "admin") requests.push(api("/api/admin/batch-image-providers"));
    const [bootstrap, providers] = await Promise.all(requests);
    data = { ...data, ...bootstrap.data, providers: providers?.data || [] };
    page.innerHTML = `${pageHeader("批量生图", `${data.jobs.length} 个任务`, `<button class="button secondary" id="refresh-batches">刷新</button><button class="button" id="create-batch">创建任务</button>`)}<section class="metric-grid batch-metrics" id="batch-metrics">${metrics()}</section>${filtersMarkup()}<section id="batch-job-list">${jobTable(data.jobs)}</section><div class="batch-load-more">${data.has_more ? '<button class="button secondary" id="load-more-batches">加载更多</button>' : ""}</div>${providerPanel()}`;
    attachPage(page);
    schedulePoll(page);
  }

  function attachPage(page) {
    page.querySelector("#create-batch")?.addEventListener("click", () => openCreate());
    page.querySelector("#empty-create-batch")?.addEventListener("click", () => openCreate());
    page.querySelector("#refresh-batches")?.addEventListener("click", renderRoute);
    page.querySelector("#load-more-batches")?.addEventListener("click", loadMore);
    page.querySelector("#batch-filter-form")?.addEventListener("submit", async event => {
      event.preventDefault();
      filters = Object.fromEntries(new FormData(event.currentTarget));
      await reloadJobs(page);
    });
    page.querySelector("#clear-batch-filter")?.addEventListener("click", async () => { filters = { status: "", api_key_id: "", task_name: "", downloaded: "" }; await renderRoute(); });
    attachJobActions(page);
    page.querySelector("#add-batch-provider")?.addEventListener("click", () => openProvider());
    page.querySelector("#empty-add-provider")?.addEventListener("click", () => openProvider());
    page.querySelectorAll("[data-provider-action]").forEach(button => button.addEventListener("click", handleProvider));
  }

  function attachJobActions(root) {
    root.querySelectorAll("[data-batch-action]").forEach(button => button.addEventListener("click", handleJob));
  }

  function listQuery(cursor = null) {
    const query = new URLSearchParams({ limit: "20" });
    Object.entries(filters).forEach(([key, value]) => { if (value) query.set(key, value); });
    if (cursor) query.set("cursor", cursor);
    return query;
  }

  async function reloadJobs(page) {
    const result = await api(`/api/user/batch-images/jobs?${listQuery()}`);
    data.jobs = result.data;
    data.has_more = result.has_more;
    data.next_cursor = result.next_cursor;
    const target = page.querySelector("#batch-job-list");
    target.innerHTML = jobTable(data.jobs);
    attachJobActions(target);
    page.querySelector(".batch-load-more").innerHTML = data.has_more ? '<button class="button secondary" id="load-more-batches">加载更多</button>' : "";
    page.querySelector("#load-more-batches")?.addEventListener("click", loadMore);
    schedulePoll(page);
  }

  async function loadMore(event) {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      const result = await api(`/api/user/batch-images/jobs?${listQuery(data.next_cursor)}`);
      data.jobs.push(...result.data);
      data.has_more = result.has_more;
      data.next_cursor = result.next_cursor;
      const page = document.querySelector("#page");
      page.querySelector("#batch-job-list").innerHTML = jobTable(data.jobs);
      attachJobActions(page.querySelector("#batch-job-list"));
      page.querySelector(".batch-load-more").innerHTML = data.has_more ? '<button class="button secondary" id="load-more-batches">加载更多</button>' : "";
      page.querySelector("#load-more-batches")?.addEventListener("click", loadMore);
    } catch (error) { toast(error.message, true); button.disabled = false; }
  }

  function schedulePoll(page) {
    clearTimeout(pollTimer);
    if (!data.jobs.some(job => !terminal.has(job.status))) return;
    pollTimer = setTimeout(async () => {
      if (currentRouteName() !== "batchImages" || !page.isConnected) return;
      try {
        const bootstrap = await api("/api/user/batch-images/bootstrap");
        data = { ...data, ...bootstrap.data };
        page.querySelector("#batch-metrics").innerHTML = metrics();
        const list = page.querySelector("#batch-job-list");
        list.innerHTML = jobTable(data.jobs);
        attachJobActions(list);
      } catch (_) {}
      schedulePoll(page);
    }, 8000);
  }

  async function handleJob(event) {
    const { batchAction: action, id } = event.currentTarget.dataset;
    if (action === "download") return download(`/api/user/batch-images/jobs/${encodeURIComponent(id)}/download`);
    if (action === "detail") return openDetail(id);
    if (action === "cancel" && !confirm("确认取消此批任务？未结算的冻结余额将被释放。")) return;
    event.currentTarget.disabled = true;
    try {
      await api(`/api/user/batch-images/jobs/${encodeURIComponent(id)}/cancel`, { method: "POST" });
      toast("任务已取消");
      await renderRoute();
    } catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
  }

  async function openDetail(id) {
    const [job, items] = await Promise.all([
      api(`/api/user/batch-images/jobs/${encodeURIComponent(id)}`),
      api(`/api/user/batch-images/jobs/${encodeURIComponent(id)}/items?limit=500`),
    ]);
    const actions = `${job.status === "completed" && job.success_count ? '<button class="button secondary" id="batch-detail-download">下载 ZIP</button><button class="button danger" id="batch-detail-clean">清理输出</button>' : ""}${job.status === "failed" && items.data.some(item => item.status === "failed") ? '<button class="button secondary" id="batch-detail-retry">重试失败项</button>' : ""}${terminal.has(job.status) ? '<button class="button danger" id="batch-detail-delete">删除记录</button>' : '<button class="button danger" id="batch-detail-cancel">取消任务</button>'}<button class="button" data-close-modal>关闭</button>`;
    openModal(job.task_name, `<div class="batch-detail-summary"><div><span>状态</span><strong>${jobStatus(job.status)}</strong></div><div><span>模型</span><strong class="mono">${escapeHtml(job.model)}</strong></div><div><span>图片</span><strong>${job.generated_image_count} / ${job.requested_image_count}</strong></div><div><span>实际金额</span><strong>${job.actual_cost_cents == null ? "-" : formatMoney(job.actual_cost_cents)}</strong></div></div><dl class="detail-list compact-detail"><div><dt>任务 ID</dt><dd class="mono">${escapeHtml(job.id)}</dd></div><div><dt>提供商</dt><dd>${escapeHtml(job.provider_name)}</dd></div><div><dt>创建时间</dt><dd>${time(job.created_at)}</dd></div>${job.parent_batch_id ? `<div><dt>父任务</dt><dd class="mono">${escapeHtml(job.parent_batch_id)}</dd></div>` : ""}${job.error ? `<div><dt>错误</dt><dd class="text-danger">${escapeHtml(job.error.code)} · ${escapeHtml(job.error.message)}</dd></div>` : ""}</dl><section class="batch-item-section"><div class="section-title"><h2>任务条目</h2><span>${items.data.length} 条</span></div>${itemTable(job, items.data)}</section>`, actions);
    modal.classList.add("batch-detail-modal");
    modal.querySelector("#batch-detail-download")?.addEventListener("click", () => download(`/api/user/batch-images/jobs/${encodeURIComponent(id)}/download`));
    modal.querySelector("#batch-detail-cancel")?.addEventListener("click", async () => mutateDetail(id, "cancel", "POST"));
    modal.querySelector("#batch-detail-delete")?.addEventListener("click", async () => { if (confirm("确认隐藏此任务记录？")) await mutateDetail(id, "", "DELETE"); });
    modal.querySelector("#batch-detail-clean")?.addEventListener("click", async () => { if (confirm("确认永久删除本地与上游输出文件？")) await mutateDetail(id, "outputs", "DELETE"); });
    modal.querySelector("#batch-detail-retry")?.addEventListener("click", () => { closeModal(); openCreate({ parent: id, ids: items.data.filter(item => item.status === "failed").map(item => item.custom_id), model: job.model }); });
    modal.querySelectorAll("[data-preview-image]").forEach(button => button.addEventListener("click", previewImage));
  }

  function itemTable(job, items) {
    if (!items.length) return emptyState("暂无条目", "任务尚未生成条目");
    return `<div class="table-wrap"><table class="batch-item-table"><thead><tr><th>条目</th><th>状态</th><th>输出</th><th>错误</th></tr></thead><tbody>${items.map(item => `<tr><td><span class="cell-main mono">${escapeHtml(item.custom_id)}</span><span class="cell-sub mono">${escapeHtml(item.prompt_hash.slice(0, 12))}</span></td><td>${itemStatus(item.status)}</td><td>${item.status === "succeeded" ? Array.from({ length: item.image_count }, (_, index) => `<button class="button quiet small" data-preview-image data-batch="${escapeHtml(job.id)}" data-custom="${escapeHtml(item.custom_id)}" data-index="${index}">预览 ${index + 1}</button>`).join("") : "-"}</td><td>${item.error ? `<span class="cell-main text-danger">${escapeHtml(item.error.code)}</span><span class="cell-sub">${escapeHtml(item.error.message)}</span>` : "-"}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function previewImage(event) {
    const button = event.currentTarget;
    const source = `/api/user/batch-images/jobs/${encodeURIComponent(button.dataset.batch)}/items/${encodeURIComponent(button.dataset.custom)}/content?image_index=${button.dataset.index}`;
    openModal(button.dataset.custom, `<div class="batch-image-preview"><img src="${source}" alt="${escapeHtml(button.dataset.custom)}"></div>`, `<a class="button secondary" href="${source}" download>下载</a><button class="button" data-close-modal>关闭</button>`);
    modal.classList.add("batch-preview-modal");
  }

  async function mutateDetail(id, suffix, method) {
    const button = document.activeElement;
    if (button) button.disabled = true;
    try {
      await api(`/api/user/batch-images/jobs/${encodeURIComponent(id)}${suffix ? `/${suffix}` : ""}`, { method });
      closeModal(); toast(method === "DELETE" ? "操作已完成" : "任务已取消"); await renderRoute();
    } catch (error) { toast(error.message, true); if (button) button.disabled = false; }
  }

  function openCreate(seed = {}) {
    const keys = enabledKeys();
    if (!keys.length) return toast("请先创建并启用 API Key", true);
    const keyId = keys[0].id;
    const models = modelsForKey(keyId);
    if (!models.length) return toast("当前 API Key 没有可用的批量图片模型", true);
    const ids = seed.ids?.length ? seed.ids : [""];
    itemSequence = 0;
    openModal(seed.parent ? "重试批任务" : "创建批任务", `<form id="batch-create-form"><div class="form-grid"><div class="field"><label for="batch-task-name">任务名称</label><input id="batch-task-name" name="task_name" maxlength="255" required></div><div class="field"><label for="batch-create-key">API Key</label><select id="batch-create-key" name="api_key_id">${keys.map(key => `<option value="${key.id}">${escapeHtml(key.name)} · ${escapeHtml(key.token_prefix)}</option>`).join("")}</select></div></div><div class="form-grid"><div class="field"><label for="batch-create-model">模型</label><select id="batch-create-model" name="model"></select></div><div class="field"><label for="batch-create-mime">输出格式</label><select id="batch-create-mime" name="response_mime_type"><option value="image/png">PNG</option><option value="image/jpeg">JPEG</option><option value="image/webp">WebP</option></select></div></div><input type="hidden" name="parent_batch_id" value="${escapeHtml(seed.parent || "")}"><div class="batch-estimate" id="batch-estimate"></div><div class="section-title batch-item-heading"><h2>Prompt</h2><button class="button secondary small" type="button" id="add-batch-item">添加</button></div><div id="batch-item-editor">${ids.map((id, index) => itemEditor(index, id)).join("")}</div><p class="form-error" id="batch-create-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="submit-batch">提交任务</button>`);
    modal.classList.add("batch-create-modal");
    const form = modal.querySelector("#batch-create-form");
    const key = form.querySelector("#batch-create-key");
    const model = form.querySelector("#batch-create-model");
    const updateModels = () => {
      const available = modelsForKey(key.value);
      model.innerHTML = available.map(item => `<option value="${escapeHtml(item.id)}" ${item.id === seed.model ? "selected" : ""}>${escapeHtml(item.id)} · ${escapeHtml(item.provider_name)}</option>`).join("");
      updateEstimate(form);
    };
    key.addEventListener("change", updateModels);
    model.addEventListener("change", () => updateEstimate(form));
    form.addEventListener("input", () => updateEstimate(form));
    form.querySelector("#add-batch-item").addEventListener("click", () => {
      const editor = form.querySelector("#batch-item-editor");
      const index = editor.children.length;
      editor.insertAdjacentHTML("beforeend", itemEditor(index, ""));
      attachItemEditors(form); updateEstimate(form);
    });
    attachItemEditors(form); updateModels();
    modal.querySelector("#submit-batch").addEventListener("click", () => submitCreate(form));
  }

  function itemEditor(index, id) {
    const uid = ++itemSequence;
    return `<article class="batch-item-editor" data-item-index="${index}"><header><strong>条目 ${index + 1}</strong><button class="button quiet small" type="button" data-remove-batch-item>移除</button></header><div class="form-grid"><div class="field"><label for="batch-custom-${uid}">自定义 ID</label><input id="batch-custom-${uid}" name="custom_id" value="${escapeHtml(id)}" maxlength="240" placeholder="自动生成"></div><div class="field"><label for="batch-count-${uid}">输出数量</label><input id="batch-count-${uid}" name="output_count" type="number" min="1" max="4" value="1" required></div></div><div class="field"><label for="batch-prompt-${uid}">Prompt</label><textarea id="batch-prompt-${uid}" name="prompt" maxlength="8000" required></textarea></div><div class="field"><label for="batch-reference-${uid}">参考图片</label><input id="batch-reference-${uid}" name="reference_images" type="file" accept="image/png,image/jpeg,image/webp" multiple></div></article>`;
  }

  function attachItemEditors(form) {
    form.querySelectorAll("[data-remove-batch-item]").forEach(button => {
      if (button.dataset.bound) return;
      button.dataset.bound = "1";
      button.addEventListener("click", () => {
        const editor = form.querySelector("#batch-item-editor");
        if (editor.children.length === 1) return toast("至少保留一个 Prompt", true);
        button.closest(".batch-item-editor").remove();
        [...editor.children].forEach((item, index) => item.querySelector("header strong").textContent = `条目 ${index + 1}`);
        updateEstimate(form);
      });
    });
  }

  function updateEstimate(form) {
    const model = modelsForKey(form.querySelector("#batch-create-key").value).find(item => item.id === form.querySelector("#batch-create-model").value);
    const count = [...form.querySelectorAll('[name="output_count"]')].reduce((sum, input) => sum + Math.max(1, Number(input.value) || 1), 0);
    if (!model) return form.querySelector("#batch-estimate").textContent = "";
    const billed = Math.ceil(model.unit_price_cents * model.batch_discount_bps / 10000) * count;
    const hold = Math.ceil(model.unit_price_cents * model.hold_bps / 10000) * count;
    form.querySelector("#batch-estimate").innerHTML = `<span>${count} 张</span><strong>预计 ${formatMoney(billed)}</strong><span>预占 ${formatMoney(hold)}</span>`;
  }

  async function fileReference(file) {
    if (file.size > 10 * 1024 * 1024) throw new Error(`${file.name} 超过 10 MiB`);
    const data = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(",", 2)[1] || "");
      reader.onerror = () => reject(new Error(`无法读取 ${file.name}`));
      reader.readAsDataURL(file);
    });
    return { id: crypto.randomUUID(), type: "inline", mime_type: file.type, data, file_uri: "" };
  }

  async function submitCreate(form) {
    const button = modal.querySelector("#submit-batch");
    const error = form.querySelector("#batch-create-error");
    if (!form.reportValidity()) return;
    button.disabled = true; error.textContent = "";
    try {
      const items = [];
      let referenceBytes = 0;
      for (const row of form.querySelectorAll(".batch-item-editor")) {
        const files = [...row.querySelector('[name="reference_images"]').files];
        referenceBytes += files.reduce((sum, file) => sum + file.size, 0) * Number(row.querySelector('[name="output_count"]').value);
        if (referenceBytes > 32 * 1024 * 1024) throw new Error("参考图片合计超过 32 MiB");
        items.push({ custom_id: row.querySelector('[name="custom_id"]').value, prompt: row.querySelector('[name="prompt"]').value, output_count: Number(row.querySelector('[name="output_count"]').value), reference_images: await Promise.all(files.map(fileReference)) });
      }
      const values = Object.fromEntries(new FormData(form));
      const payload = { api_key_id: Number(values.api_key_id), idempotency_key: crypto.randomUUID(), model: values.model, task_name: values.task_name, parent_batch_id: values.parent_batch_id, provider: "", response_mime_type: values.response_mime_type, image_size: "1K", items, metadata: {} };
      const result = await api("/api/user/batch-images/jobs", { method: "POST", body: JSON.stringify(payload) });
      closeModal(); toast(`任务 ${result.id} 已提交`); await renderRoute();
    } catch (requestError) { error.textContent = requestError.message; button.disabled = false; }
  }

  function download(path) {
    const link = document.createElement("a");
    link.href = path; link.download = ""; document.body.append(link); link.click(); link.remove();
  }

  function providerPayload(provider) {
    return { name: provider.name, kind: provider.kind, base_url: provider.base_url, api_key: null, service_account_json: null, project_id: provider.project_id || "", location: provider.location || "global", gcs_bucket: provider.gcs_bucket || "", gcs_prefix: provider.gcs_prefix || "batch-image/mini/{batch_id}", gcs_base_url: provider.gcs_base_url || "https://storage.googleapis.com", token_url: provider.token_url || "https://oauth2.googleapis.com/token", models: provider.models, unit_price_cents: provider.unit_price_cents, batch_discount_bps: provider.batch_discount_bps, hold_bps: provider.hold_bps, priority: provider.priority, concurrency: provider.concurrency, enabled: provider.enabled };
  }

  function openProvider(provider = null) {
    const item = provider || { name: "", kind: "gemini_api", base_url: "https://generativelanguage.googleapis.com", project_id: "", location: "global", gcs_bucket: "", gcs_prefix: "batch-image/mini/{batch_id}", gcs_base_url: "https://storage.googleapis.com", token_url: "https://oauth2.googleapis.com/token", models: ["gemini-3.1-flash-image"], unit_price_cents: 0, batch_discount_bps: 5000, hold_bps: 6000, priority: 50, concurrency: 1, enabled: true };
    openModal(provider ? "编辑图片提供商" : "添加图片提供商", `<form id="batch-provider-form"><div class="form-grid"><div class="field"><label for="batch-provider-name">名称</label><input id="batch-provider-name" name="name" value="${escapeHtml(item.name)}" maxlength="100" required></div><div class="field"><label for="batch-provider-kind">类型</label><select id="batch-provider-kind" name="kind"><option value="gemini_api" ${item.kind !== "vertex" ? "selected" : ""}>Gemini API</option><option value="vertex" ${item.kind === "vertex" ? "selected" : ""}>Vertex</option></select></div></div><div data-gemini-provider><div class="field"><label for="batch-provider-key">Gemini API Key</label><input id="batch-provider-key" name="api_key" type="password" autocomplete="new-password" placeholder="${provider?.has_api_key ? "留空保留已保存密钥" : ""}"></div></div><div data-vertex-provider><div class="field"><label for="batch-provider-service-account">Service Account JSON</label><input id="batch-provider-service-account" name="service_account_file" type="file" accept="application/json,.json"></div><div class="form-grid"><div class="field"><label for="batch-provider-project">Project ID</label><input id="batch-provider-project" name="project_id" value="${escapeHtml(item.project_id || "")}" maxlength="128"></div><div class="field"><label for="batch-provider-location">Location</label><input id="batch-provider-location" name="location" value="${escapeHtml(item.location || "global")}" maxlength="63" required></div></div><div class="form-grid"><div class="field"><label for="batch-provider-bucket">GCS Bucket</label><input id="batch-provider-bucket" name="gcs_bucket" value="${escapeHtml(item.gcs_bucket || "")}" maxlength="222" required></div><div class="field"><label for="batch-provider-prefix">GCS Prefix</label><input id="batch-provider-prefix" name="gcs_prefix" value="${escapeHtml(item.gcs_prefix || "batch-image/mini/{batch_id}")}" maxlength="512" required></div></div><div class="form-grid"><div class="field"><label for="batch-provider-gcs-url">GCS Base URL</label><input id="batch-provider-gcs-url" name="gcs_base_url" type="url" value="${escapeHtml(item.gcs_base_url || "https://storage.googleapis.com")}" required></div><div class="field"><label for="batch-provider-token-url">Token URL</label><input id="batch-provider-token-url" name="token_url" type="url" value="${escapeHtml(item.token_url || "https://oauth2.googleapis.com/token")}" required></div></div></div><div class="field"><label for="batch-provider-url">API Base URL</label><input id="batch-provider-url" name="base_url" type="url" value="${escapeHtml(item.base_url || "")}" placeholder="留空使用官方端点"></div><div class="field"><label for="batch-provider-models">模型</label><textarea id="batch-provider-models" name="models" class="compact-textarea" required>${escapeHtml(item.models.join("\n"))}</textarea></div><div class="form-grid batch-provider-numbers"><div class="field"><label for="batch-provider-price">单价（分/张）</label><input id="batch-provider-price" name="unit_price_cents" type="number" min="0" max="1000000" value="${item.unit_price_cents}" required></div><div class="field"><label for="batch-provider-discount">批折扣（%）</label><input id="batch-provider-discount" name="batch_discount" type="number" min="0" max="100" value="${item.batch_discount_bps / 100}" required></div><div class="field"><label for="batch-provider-hold">冻结比例（%）</label><input id="batch-provider-hold" name="hold" type="number" min="0" max="100" value="${item.hold_bps / 100}" required></div><div class="field"><label for="batch-provider-priority">优先级</label><input id="batch-provider-priority" name="priority" type="number" min="0" max="10000" value="${item.priority}" required></div><div class="field"><label for="batch-provider-concurrency">并发</label><input id="batch-provider-concurrency" name="concurrency" type="number" min="1" max="16" value="${item.concurrency}" required></div><label class="toggle-line"><input name="enabled" type="checkbox" ${item.enabled ? "checked" : ""}> 启用</label></div><p class="form-error" id="batch-provider-error"></p></form>`, `<button class="button secondary" data-close-modal>取消</button><button class="button" id="save-batch-provider">保存</button>`);
    const form = modal.querySelector("#batch-provider-form");
    const updateKind = () => {
      const vertex = form.elements.kind.value === "vertex";
      form.querySelector("[data-gemini-provider]").hidden = vertex;
      form.querySelector("[data-vertex-provider]").hidden = !vertex;
      form.elements.api_key.required = !vertex && !provider?.has_api_key;
      form.elements.service_account_file.required = vertex && !provider?.has_service_account;
      if (!vertex && !form.elements.base_url.value) form.elements.base_url.value = "https://generativelanguage.googleapis.com";
      if (vertex && form.elements.base_url.value === "https://generativelanguage.googleapis.com") form.elements.base_url.value = "";
    };
    form.elements.kind.addEventListener("change", updateKind);
    updateKind();
    modal.querySelector("#save-batch-provider").addEventListener("click", async event => {
      if (!form.reportValidity()) return;
      event.currentTarget.disabled = true;
      const values = Object.fromEntries(new FormData(form));
      try {
        const file = form.elements.service_account_file.files[0];
        if (file && file.size > 128 * 1024) throw new Error("Service Account JSON 超过 128 KiB");
        const serviceAccount = file ? await file.text() : null;
        if (serviceAccount && !values.project_id) values.project_id = JSON.parse(serviceAccount).project_id || "";
        const payload = { name: values.name, kind: values.kind, base_url: values.base_url, api_key: values.api_key || null, service_account_json: serviceAccount, project_id: values.project_id || "", location: values.location || "global", gcs_bucket: values.gcs_bucket || "", gcs_prefix: values.gcs_prefix || "batch-image/mini/{batch_id}", gcs_base_url: values.gcs_base_url || "https://storage.googleapis.com", token_url: values.token_url || "https://oauth2.googleapis.com/token", models: parseModelList(values.models), unit_price_cents: Number(values.unit_price_cents), batch_discount_bps: Math.round(Number(values.batch_discount) * 100), hold_bps: Math.round(Number(values.hold) * 100), priority: Number(values.priority), concurrency: Number(values.concurrency), enabled: form.elements.enabled.checked };
        await api(provider ? `/api/admin/batch-image-providers/${provider.id}` : "/api/admin/batch-image-providers", { method: provider ? "PUT" : "POST", body: JSON.stringify(payload) }); closeModal(); toast("图片提供商已保存"); await renderRoute();
      }
      catch (error) { form.querySelector("#batch-provider-error").textContent = error.message; event.currentTarget.disabled = false; }
    });
  }

  async function handleProvider(event) {
    const id = Number(event.currentTarget.dataset.id);
    const provider = data.providers.find(item => item.id === id);
    const action = event.currentTarget.dataset.providerAction;
    if (!provider) return;
    if (action === "edit") return openProvider(provider);
    if (action === "delete" && !confirm("确认删除此提供商？已有任务引用时请改为停用。")) return;
    event.currentTarget.disabled = true;
    try {
      if (action === "test") {
        const result = await api(`/api/admin/batch-image-providers/${id}/test`, { method: "POST" });
        toast(`连接正常 · ${result.data.latency_ms} ms`);
      } else if (action === "toggle") {
        await api(`/api/admin/batch-image-providers/${id}`, { method: "PUT", body: JSON.stringify({ ...providerPayload(provider), enabled: !provider.enabled }) });
        toast("提供商状态已更新"); await renderRoute();
      } else {
        await api(`/api/admin/batch-image-providers/${id}`, { method: "DELETE" });
        toast("提供商已删除"); await renderRoute();
      }
    } catch (error) { toast(error.message, true); event.currentTarget.disabled = false; }
  }

  return { render };
})();
