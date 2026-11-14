import { describe, expect, it } from "vitest";
import {
  NO_FILTER,
  categoryOf,
  entryId,
  extensionOf,
  filterEntries,
  modeOf,
  moveFocus,
  parseExtensions,
  pruneSelection,
  select,
  selectAll,
  shortcutOf,
  sortEntries,
  typeLabel,
  type Entry,
} from "./fileList";

const f = (name: string, size: number, lastModified: string | null = null, key = `p/${name}`): Entry => ({
  kind: "file",
  name,
  key,
  size,
  lastModified,
});
const d = (name: string): Entry => ({ kind: "folder", name, prefix: `p/${name}/` });

describe("sorting", () => {
  const entries = [f("b10.txt", 5, "2024-01-02T00:00:00Z"), d("zeta"), f("b2.txt", 50, "2023-01-01T00:00:00Z"), d("Alpha"), f("a.PDF", 1)];

  it("puts folders first and sorts names naturally and case-insensitively", () => {
    expect(sortEntries(entries, "name", "ascending").map((e) => e.name)).toEqual(["Alpha", "zeta", "a.PDF", "b2.txt", "b10.txt"]);
    expect(sortEntries(entries, "name", "descending").map((e) => e.name)).toEqual(["zeta", "Alpha", "b10.txt", "b2.txt", "a.PDF"]);
  });

  it("sorts by size, date and type, folders still first", () => {
    expect(sortEntries(entries, "size", "descending").map((e) => e.name)).toEqual(["Alpha", "zeta", "b2.txt", "b10.txt", "a.PDF"]);
    expect(sortEntries(entries, "modified", "ascending").map((e) => e.name)).toEqual(["Alpha", "zeta", "a.PDF", "b2.txt", "b10.txt"]);
    expect(sortEntries(entries, "type", "ascending").map((e) => e.name)).toEqual(["Alpha", "zeta", "a.PDF", "b2.txt", "b10.txt"]);
  });

  it("keeps the provider's order by default, folders first", () => {
    expect(sortEntries(entries, "listed", "ascending").map((e) => e.name)).toEqual(["zeta", "Alpha", "b10.txt", "b2.txt", "a.PDF"]);
    expect(sortEntries(entries, "listed", "descending").map((e) => e.name)).toEqual(["Alpha", "zeta", "a.PDF", "b2.txt", "b10.txt"]);
  });

  it("is stable for equal names in different folders (search results)", () => {
    const same = [f("x", 1, null, "b/x"), f("x", 1, null, "a/x")];
    expect(sortEntries(same, "name", "ascending").map(entryId)).toEqual(["a/x", "b/x"]);
  });
});

describe("types and filters", () => {
  it("classifies by extension", () => {
    expect(extensionOf("photo.JPG")).toBe("jpg");
    expect(extensionOf(".env")).toBe("");
    expect(extensionOf("noext")).toBe("");
    expect(categoryOf(f("a.jpg", 1))).toBe("image");
    expect(categoryOf(f("a.weird", 1))).toBe("other");
    expect(categoryOf(d("x"))).toBe("folder");
    expect(typeLabel(f("a.tar.gz", 1))).toBe("GZ file");
    expect(typeLabel(f("README", 1))).toBe("File");
  });

  const entries = [d("docs"), f("a.pdf", 10, "2024-05-01T00:00:00Z"), f("b.jpg", 2000, "2024-06-01T00:00:00Z"), f("c.txt", 300, null)];

  it("shows only files or only folders", () => {
    expect(filterEntries(entries, { ...NO_FILTER, show: "folders" }).map((e) => e.name)).toEqual(["docs"]);
    expect(filterEntries(entries, { ...NO_FILTER, show: "files" })).toHaveLength(3);
  });

  it("filters by category, extension, size and date; folders drop out of file-only filters", () => {
    expect(filterEntries(entries, { ...NO_FILTER, categories: ["image"] }).map((e) => e.name)).toEqual(["b.jpg"]);
    expect(filterEntries(entries, { ...NO_FILTER, extensions: ["pdf", "txt"] }).map((e) => e.name)).toEqual(["a.pdf", "c.txt"]);
    expect(filterEntries(entries, { ...NO_FILTER, minSize: 100, maxSize: 1000 }).map((e) => e.name)).toEqual(["c.txt"]);
    expect(
      filterEntries(entries, { ...NO_FILTER, modifiedFrom: Date.parse("2024-05-15T00:00:00Z") }).map((e) => e.name),
    ).toEqual(["b.jpg"]);
    expect(filterEntries(entries, NO_FILTER)).toHaveLength(4);
  });

  it("parses typed extensions", () => {
    expect(parseExtensions(" .PDF, jpg;;docx  pdf")).toEqual(["pdf", "jpg", "docx"]);
    expect(parseExtensions("")).toEqual([]);
  });
});

describe("selection", () => {
  const order = ["a", "b", "c", "d", "e"];

  it("replaces, toggles and selects ranges from the anchor", () => {
    let s = select({ ids: new Set(), anchor: null }, "b", "replace", order);
    expect([...s.ids]).toEqual(["b"]);
    s = select(s, "d", "toggle", order);
    expect([...s.ids].sort()).toEqual(["b", "d"]);
    s = select(s, "d", "toggle", order);
    expect([...s.ids]).toEqual(["b"]);
    s = select({ ids: new Set(["b"]), anchor: "b" }, "e", "range", order);
    expect([...s.ids]).toEqual(["b", "c", "d", "e"]);
    expect(s.anchor).toBe("b");
    s = select(s, "a", "range", order);
    expect([...s.ids]).toEqual(["a", "b"]);
    s = select({ ids: new Set(["e"]), anchor: "a" }, "b", "add-range", order);
    expect([...s.ids].sort()).toEqual(["a", "b", "e"]);
  });

  it("maps modifier keys to modes", () => {
    expect(modeOf({ ctrlKey: false, metaKey: false, shiftKey: false })).toBe("replace");
    expect(modeOf({ ctrlKey: true, metaKey: false, shiftKey: false })).toBe("toggle");
    expect(modeOf({ ctrlKey: false, metaKey: true, shiftKey: false })).toBe("toggle");
    expect(modeOf({ ctrlKey: false, metaKey: false, shiftKey: true })).toBe("range");
    expect(modeOf({ ctrlKey: true, metaKey: false, shiftKey: true })).toBe("add-range");
  });

  it("selects all visible, prunes vanished ids and moves the focus", () => {
    expect([...selectAll(order).ids]).toEqual(order);
    const pruned = pruneSelection({ ids: new Set(["a", "x"]), anchor: "x" }, order);
    expect([...pruned.ids]).toEqual(["a"]);
    expect(pruned.anchor).toBeNull();
    expect(moveFocus(order, null, 1)).toBe("a");
    expect(moveFocus(order, null, -1)).toBe("e");
    expect(moveFocus(order, "e", 1)).toBe("e");
    expect(moveFocus(order, "c", -1)).toBe("b");
    expect(moveFocus([], null, 1)).toBeNull();
  });
});

describe("shortcuts", () => {
  const key = (k: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean }> = {}, target: EventTarget | null = null) => ({
    key: k,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    target,
    ...mods,
  });

  it("maps the file-manager keys", () => {
    expect(shortcutOf(key("a", { ctrlKey: true }))).toBe("select-all");
    expect(shortcutOf(key("C", { ctrlKey: true }))).toBe("copy");
    expect(shortcutOf(key("x", { metaKey: true }))).toBe("cut");
    expect(shortcutOf(key("v", { ctrlKey: true }))).toBe("paste");
    expect(shortcutOf(key("r", { ctrlKey: true }))).toBe("refresh");
    expect(shortcutOf(key("Delete"))).toBe("delete");
    expect(shortcutOf(key("F2"))).toBe("rename");
    expect(shortcutOf(key("Escape"))).toBe("escape");
    expect(shortcutOf(key("ArrowDown", { shiftKey: true }))).toBe("extend-down");
    expect(shortcutOf(key("a"))).toBeNull();
    expect(shortcutOf(key("a", { ctrlKey: true, altKey: true }))).toBeNull();
  });

  it("never takes keys from a text field, but still Escape", () => {
    const input = document.createElement("input");
    input.type = "text";
    const box = document.createElement("input");
    box.type = "checkbox";
    const area = document.createElement("textarea");
    expect(shortcutOf(key("a", { ctrlKey: true }, input))).toBeNull();
    expect(shortcutOf(key("Delete", {}, area))).toBeNull();
    expect(shortcutOf(key("Escape", {}, input))).toBe("escape");
    expect(shortcutOf(key("a", { ctrlKey: true }, box))).toBe("select-all");
  });
});
