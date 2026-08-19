type Catalog = Record<string, string>;

export const AVAILABLE = ["en", "ja"] as const;
export type LangCode = (typeof AVAILABLE)[number];

const DEFAULT_LANG: LangCode = "en";
const STORAGE_KEY = "lang";

const catalogs: Record<string, Catalog> = {};
let current: LangCode = DEFAULT_LANG;
const listeners = new Set<() => void>();

function isAvailable(code: string): code is LangCode {
  return (AVAILABLE as readonly string[]).includes(code);
}

/** Explicit choice first, then the browser's preference. */
function detect(): LangCode {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && isAvailable(stored)) return stored;
  const code = navigator.language.toLowerCase().split("-")[0];
  return isAvailable(code) ? code : DEFAULT_LANG;
}

async function fetchCatalog(code: string): Promise<Catalog> {
  const res = await fetch(`/locales/${code}.json`);
  if (!res.ok) throw new Error(`locale ${code}: ${res.status}`);
  return res.json();
}

export function getLang(): LangCode {
  return current;
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Loads the fallback catalog plus the detected one, before the first render. */
export async function init(): Promise<void> {
  const detected = detect();
  if (detected === DEFAULT_LANG) {
    catalogs[DEFAULT_LANG] = await fetchCatalog(DEFAULT_LANG);
  } else {
    const [fallback, local] = await Promise.all([
      fetchCatalog(DEFAULT_LANG),
      fetchCatalog(detected).catch(() => null),
    ]);
    catalogs[DEFAULT_LANG] = fallback;
    if (local) {
      catalogs[detected] = local;
      current = detected;
    }
  }
  document.documentElement.lang = current;
}

export async function setLang(code: LangCode): Promise<void> {
  if (!catalogs[code]) catalogs[code] = await fetchCatalog(code);
  current = code;
  localStorage.setItem(STORAGE_KEY, code);
  document.documentElement.lang = code;
  for (const listener of listeners) listener();
}

export function has(key: string): boolean {
  return key in (catalogs[current] ?? {}) || key in (catalogs[DEFAULT_LANG] ?? {});
}

export function t(key: string, params?: Record<string, string | number>): string {
  let message = catalogs[current]?.[key] ?? catalogs[DEFAULT_LANG]?.[key] ?? key;
  if (params) {
    for (const [name, value] of Object.entries(params)) {
      message = message.replaceAll(`{${name}}`, String(value));
    }
  }
  return message;
}
