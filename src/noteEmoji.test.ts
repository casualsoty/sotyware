// The notes overlay's custom emojis (ADR-052): which pictures make the
// set, and what the menu types.
import { describe, expect, it } from "vitest";
import { EMOJI_ROWS, emojiInsert, emojiSet, PROFESSIONS } from "./noteEmoji";

describe("the emoji set", () => {
  it("offers the icons whose picture is bundled, a row per profession in the game's order", () => {
    const set = emojiSet({
      "./assets/emoji/gw2/reaper.png": "/a/reaper.png",
      "./assets/emoji/gw2/thief.png": "/a/thief-123.png",
      "./assets/emoji/gw2/guardian.svg": "/a/guardian-456.svg",
      "./assets/emoji/gw2/unknown.png": "/a/unknown.png",
      "./assets/emoji/gw2/README.md": "/a/README.md",
    });
    expect(set).toEqual([
      [{ id: "guardian", name: "Guardian", src: "/a/guardian-456.svg" }],
      [{ id: "thief", name: "Thief", src: "/a/thief-123.png" }],
      [{ id: "reaper", name: "Reaper", src: "/a/reaper.png" }],
    ]);
  });

  it("bundles every profession and elite specialization, nothing else", () => {
    expect(EMOJI_ROWS.map((row) => row.map((e) => e.name))).toEqual(PROFESSIONS);
    expect(EMOJI_ROWS.flat()).toHaveLength(45);
  });

  it("has shortcodes the parser reads: lowercase letters, each once", () => {
    const ids = PROFESSIONS.flat().map((name) => name.toLowerCase());
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) {
      expect(id).toMatch(/^[a-z]+$/);
    }
  });
});

describe("what the menu types", () => {
  it("separates the shortcode from a word or another emoji before the caret", () => {
    expect(emojiInsert("", 0, "thief")).toBe(":thief:");
    expect(emojiInsert("Tank ", 5, "thief")).toBe(":thief:");
    expect(emojiInsert("Tank", 4, "thief")).toBe(" :thief:");
    expect(emojiInsert(":guardian:", 10, "thief")).toBe(" :thief:");
    expect(emojiInsert("(", 1, "thief")).toBe(":thief:");
  });
});
