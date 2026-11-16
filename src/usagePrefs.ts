// The downloads per 30 days the user assumes for each provider account in
// the cost estimates (ADR-041). A per-device display preference, like the
// theme: kept in the webview's own storage, never sent anywhere but to the
// estimate. Missing or refused storage only means no assumption (0 GB).

/** The storage key; versioned so a future format can ignore old values. */
export const DOWNLOADS_KEY = "sotyware.usage.downloads.v1";

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** GB per 30 days by provider account group (`ProviderGroup.key`). */
export function loadDownloads(storage: Pick<Storage, "getItem"> | null = safeStorage()): Record<string, number> {
  try {
    const raw = storage?.getItem(DOWNLOADS_KEY) ?? null;
    const value: unknown = raw === null ? null : JSON.parse(raw);
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return {};
    }
    const out: Record<string, number> = {};
    for (const [k, v] of Object.entries(value)) {
      if (typeof v === "number" && Number.isFinite(v) && v >= 0) {
        out[k] = v;
      }
    }
    return out;
  } catch {
    return {};
  }
}

/** Saves the assumption for one group; false if the webview refused. */
export function saveDownloads(
  group: string,
  gb: number,
  storage: Pick<Storage, "getItem" | "setItem"> | null = safeStorage(),
): boolean {
  try {
    if (storage === null) {
      return false;
    }
    const all = loadDownloads(storage);
    if (Number.isFinite(gb) && gb > 0) {
      all[group] = gb;
    } else {
      delete all[group];
    }
    storage.setItem(DOWNLOADS_KEY, JSON.stringify(all));
    return true;
  } catch {
    return false;
  }
}
