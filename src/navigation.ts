// The dashboard's places. Navigation is local state in App.svelte: the page
// is never reloaded, and the Overview is what the dashboard opens with.

export type Section = "overview" | "cloud-storage" | "backups" | "autoclicker" | "account" | "settings";

export type SettingsTab = "general" | "features" | "overlay" | "about";

export const SECTION_TITLES: Record<Section, string> = {
  overview: "Overview",
  "cloud-storage": "Cloud Storage",
  backups: "Backups",
  autoclicker: "Autoclicker",
  account: "Account",
  settings: "Settings",
};

/** Keyboard shortcuts of the shell, with Ctrl (Windows, Linux) or ⌘
 * (macOS): 1–4 switch sections, comma opens Settings. Typing in a field is
 * never taken over: only modified keys count. */
export function shellShortcut(e: {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): Section | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) {
    return null;
  }
  switch (e.key) {
    case "1":
      return "overview";
    case "2":
      return "cloud-storage";
    case "3":
      return "backups";
    case "4":
      return "autoclicker";
    case ",":
      return "settings";
    default:
      return null;
  }
}
