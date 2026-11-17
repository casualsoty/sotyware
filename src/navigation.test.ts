// The shell's keyboard shortcuts and the colour meaning of check results.
import { describe, expect, it } from "vitest";
import { CHECK_STATUSES } from "./cloud";
import { checkTone } from "./cloudMessages";
import { shellShortcut } from "./navigation";

const none = { ctrlKey: false, metaKey: false, altKey: false, shiftKey: false };

describe("shellShortcut", () => {
  it("maps Ctrl or ⌘ with 1–4 and comma to sections", () => {
    expect(shellShortcut({ ...none, key: "1", ctrlKey: true })).toBe("overview");
    expect(shellShortcut({ ...none, key: "2", metaKey: true })).toBe("cloud-storage");
    expect(shellShortcut({ ...none, key: "3", ctrlKey: true })).toBe("backups");
    expect(shellShortcut({ ...none, key: "4", ctrlKey: true })).toBe("autoclicker");
    expect(shellShortcut({ ...none, key: ",", metaKey: true })).toBe("settings");
  });

  it("leaves every other combination to the page", () => {
    expect(shellShortcut({ ...none, key: "1" })).toBeNull();
    expect(shellShortcut({ ...none, key: "5", ctrlKey: true })).toBeNull();
    expect(shellShortcut({ ...none, key: "1", ctrlKey: true, shiftKey: true })).toBeNull();
    expect(shellShortcut({ ...none, key: "1", ctrlKey: true, altKey: true })).toBeNull();
  });
});

describe("checkTone", () => {
  it("gives every check result a meaning, green only when connected", () => {
    expect(checkTone(null)).toBe("neutral");
    for (const status of CHECK_STATUSES) {
      const tone = checkTone(status);
      expect(["success", "warning", "danger"]).toContain(tone);
      expect(tone === "success").toBe(status === "connected");
    }
    expect(checkTone("credential-store-locked")).toBe("warning");
    expect(checkTone("invalid-credentials")).toBe("danger");
  });
});
