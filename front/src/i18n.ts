type Catalog = Record<string, string>;

const AVAILABLE = ["en", "ja"] as const;
type LangCode = (typeof AVAILABLE)[number];

const DEFAULT_LANG: LangCode = "en";

const catalogs: Record<string, Catalog> = {};
let current: LangCode = DEFAULT_LANG;

function isAvailable(code: string): code is LangCode {
  return (AVAILABLE as readonly string[]).includes(code);
}

/** The browser's preferred language, if we ship one it asks for. */
function detect(): LangCode {
  const preferences = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const preference of preferences) {
    const code = preference.toLowerCase().split("-")[0];
    if (isAvailable(code)) return code;
  }
  return DEFAULT_LANG;
}

async function fetchCatalog(code: string): Promise<Catalog> {
  const res = await fetch(`/locales/${code}.json`);
  if (!res.ok) throw new Error(`locale ${code}: ${res.status}`);
  return res.json();
}

export function getLang(): LangCode {
  return current;
}

/**
 * Loads the fallback catalog plus the detected one, before the first render.
 * The language is settled here and never changes afterwards.
 */
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
