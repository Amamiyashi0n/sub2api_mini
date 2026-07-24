"use strict";
(function () {
const widgets = new WeakMap();
let loader;
function load() {
  if (window.turnstile) return Promise.resolve(window.turnstile);
  if (loader) return loader;
  loader = new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    script.async = true; script.defer = true;
    script.onload = () => window.turnstile ? resolve(window.turnstile) : reject(new Error("Turnstile API unavailable"));
    script.onerror = () => reject(new Error("Turnstile API unavailable"));
    document.head.append(script);
  });
  return loader;
}
async function mount(form, siteKey) {
  const slot = form?.querySelector("[data-turnstile-slot]");
  if (!slot || !siteKey || widgets.has(form)) return;
  const api = await load();
  const input = form.querySelector('[name="turnstile_token"]');
  const id = api.render(slot, {
    sitekey: siteKey,
    callback: token => { input.value = token; },
    "expired-callback": () => { input.value = ""; },
    "error-callback": () => { input.value = ""; },
  });
  widgets.set(form, id);
}
function token(form) { return form?.querySelector('[name="turnstile_token"]')?.value || ""; }
function reset(form) {
  const id = widgets.get(form);
  if (id != null && window.turnstile) window.turnstile.reset(id);
  const input = form?.querySelector('[name="turnstile_token"]'); if (input) input.value = "";
}
window.Sub2MiniTurnstile = { mount, token, reset };
})();
