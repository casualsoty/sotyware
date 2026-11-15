// The dashboard's colour theme (ADR-035): follow the system, or always light
// or dark. The choice is a per-device display preference: it is kept in the
// webview's own storage (no Rust command, no database), read synchronously
// before the first paint so the page never flashes the other theme, and
// mirrored to the native window so its title bar matches (Windows, macOS,
// GTK). Storage that is missing or throws (a private profile, cleared data)
// only means the system theme is used.
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ThemeChoice = "system" | "light" | "dark";

export const THEME_CHOICES: readonly { id: ThemeChoice; label: string }[] = [
  { id: "system", label: "System" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
];

/** The storage key; versioned so a future format can ignore old values. */
export const THEME_KEY = "sotyware.theme.v1";

function isChoice(value: unknown): value is ThemeChoice {
  return value === "system" || value === "light" || value === "dark";
}

/** The saved choice, or "system" when none (or an unreadable one) is stored. */
export function loadTheme(storage: Pick<Storage, "getItem"> | null = safeStorage()): ThemeChoice {
  try {
    const value = storage?.getItem(THEME_KEY) ?? null;
    return isChoice(value) ? value : "system";
  } catch {
    return "system";
  }
}

/** Saves `choice`; false if the webview refused to store it (it still
 * applies for this session). */
export function saveTheme(choice: ThemeChoice, storage: Pick<Storage, "setItem" | "removeItem"> | null = safeStorage()): boolean {
  try {
    if (storage === null) {
      return false;
    }
    if (choice === "system") {
      storage.removeItem(THEME_KEY);
    } else {
      storage.setItem(THEME_KEY, choice);
    }
    return true;
  } catch {
    return false;
  }
}

/** Sets the page's colours: `data-theme` on <html> overrides the system's
 * `prefers-color-scheme` (tokens.css); "system" removes it. */
export function applyTheme(choice: ThemeChoice, root: HTMLElement = document.documentElement): void {
  if (choice === "system") {
    delete root.dataset["theme"];
  } else {
    root.dataset["theme"] = choice;
  }
}

/** Asks the native window to use the same theme (title bar, native
 * controls); `null` follows the system. A refusal is harmless: only the
 * title bar keeps the system's colours. */
export async function applyWindowTheme(choice: ThemeChoice): Promise<void> {
  try {
    await getCurrentWindow().setTheme(choice === "system" ? null : choice);
  } catch {
    // No window (tests, the browser preview) or the permission is missing.
  }
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}
