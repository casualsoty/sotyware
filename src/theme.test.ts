// The theme preference (ADR-035): saved in the webview's storage, applied as
// `data-theme` on <html>, "system" when nothing usable is stored.
import { afterEach, describe, expect, it } from "vitest";
import { applyTheme, loadTheme, saveTheme, THEME_KEY } from "./theme";

function memory(initial: Record<string, string> = {}): Storage & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    get length(): number {
      return Object.keys(data).length;
    },
    clear: (): void => {
      for (const k of Object.keys(data)) {
        delete data[k];
      }
    },
    getItem: (k: string): string | null => data[k] ?? null,
    key: (i: number): string | null => Object.keys(data)[i] ?? null,
    removeItem: (k: string): void => {
      delete data[k];
    },
    setItem: (k: string, v: string): void => {
      data[k] = v;
    },
  };
}

const broken = {
  getItem: (): string | null => {
    throw new Error("SecurityError");
  },
  setItem: (): void => {
    throw new Error("QuotaExceededError");
  },
  removeItem: (): void => {
    throw new Error("SecurityError");
  },
};

afterEach(() => {
  delete document.documentElement.dataset["theme"];
});

describe("theme preference", () => {
  it("follows the system until a choice is saved", () => {
    expect(loadTheme(memory())).toBe("system");
    expect(loadTheme(null)).toBe("system");
  });

  it("round-trips light and dark, and forgets the key for system", () => {
    const store = memory();
    expect(saveTheme("dark", store)).toBe(true);
    expect(store.data[THEME_KEY]).toBe("dark");
    expect(loadTheme(store)).toBe("dark");
    expect(saveTheme("light", store)).toBe(true);
    expect(loadTheme(store)).toBe("light");
    expect(saveTheme("system", store)).toBe(true);
    expect(THEME_KEY in store.data).toBe(false);
    expect(loadTheme(store)).toBe("system");
  });

  it("ignores a stored value it does not know", () => {
    expect(loadTheme(memory({ [THEME_KEY]: "sepia" }))).toBe("system");
    expect(loadTheme(memory({ [THEME_KEY]: "" }))).toBe("system");
  });

  it("treats storage that throws as no storage, and says the save failed", () => {
    expect(loadTheme(broken)).toBe("system");
    expect(saveTheme("dark", broken)).toBe(false);
    expect(saveTheme("dark", null)).toBe(false);
  });

  it("sets data-theme on the root for light and dark, and removes it for system", () => {
    const root = document.documentElement;
    applyTheme("dark", root);
    expect(root.dataset["theme"]).toBe("dark");
    applyTheme("light", root);
    expect(root.dataset["theme"]).toBe("light");
    applyTheme("system", root);
    expect(root.dataset["theme"]).toBeUndefined();
  });
});
