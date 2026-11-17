// UI text comes only from locales/*.json. English is the fallback for any missing key.
const { invoke } = window.__TAURI__.core;

let english = {};
let current = {};
let code = "en";
let languages = [];
let userLocales = {};

const lookup = (obj, key) => key.split(".").reduce((o, k) => (o == null ? undefined : o[k]), obj);

async function readJson(path) {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`${path}: ${res.status}`);
  return res.json();
}

export async function initI18n(preferred) {
  const index = await readJson("locales/index.json");
  english = await readJson("locales/en.json");
  languages = index.languages.slice();
  try {
    userLocales = await invoke("user_locales");
  } catch {
    userLocales = {};
  }
  for (const [c, data] of Object.entries(userLocales)) {
    if (!languages.some((l) => l.code === c)) languages.push({ code: c, name: data?._meta?.name || c });
  }
  await setLanguage(preferred || index.default || "en");
}

export async function setLanguage(next) {
  let data = null;
  if (userLocales[next]) data = userLocales[next];
  else if (languages.some((l) => l.code === next) && next !== "en") data = await readJson(`locales/${next}.json`).catch(() => null);
  code = data ? next : "en";
  current = data || english;
  document.documentElement.lang = code;
  document.documentElement.dir = lookup(current, "_meta.direction") || "ltr";
  apply(document);
  return code;
}

export function t(key, vars = {}) {
  let v = lookup(current, key);
  if (v == null) v = lookup(english, key);
  if (v == null) return key;
  if (typeof v !== "string") return v;
  return v.replace(/\{(\w+)\}/g, (m, k) => (vars[k] != null ? String(vars[k]) : m));
}

/** Fill every [data-i18n] text and [data-i18n-attr="attr:key;attr:key"] under root. */
export function apply(root) {
  root.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  root.querySelectorAll("[data-i18n-attr]").forEach((el) => {
    for (const pair of el.dataset.i18nAttr.split(";")) {
      const [attr, key] = pair.split(":");
      if (attr && key) el.setAttribute(attr, t(key));
    }
  });
}

export const languageList = () => languages.slice();
export const languageCode = () => code;

/** A BCP 47 tag Intl accepts, falling back to English for made-up codes. */
export function intlLocale() {
  try {
    return Intl.DateTimeFormat.supportedLocalesOf([code]).length ? code : "en";
  } catch {
    return "en";
  }
}
