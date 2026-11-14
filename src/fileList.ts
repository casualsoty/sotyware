// Pure logic of the file list (ADR-026): one shape for folders and files,
// client-side sorting and filtering of the entries already loaded (never a
// request: the provider lists in key order and cannot sort or filter by
// size, date or type), the selection model (click, Ctrl-click, Shift-click,
// select all) and the keyboard shortcuts. Unit-tested in fileList.test.ts.

export type Entry =
  | { kind: "folder"; name: string; prefix: string }
  | { kind: "file"; name: string; key: string; size: number; lastModified: string | null };

/** The entry's key (file) or prefix (folder): unique within a listing. */
export function entryId(e: Entry): string {
  return e.kind === "folder" ? e.prefix : e.key;
}

export const CATEGORIES = ["image", "video", "audio", "document", "spreadsheet", "archive", "code", "text", "other"] as const;
export type Category = (typeof CATEGORIES)[number];

const BY_EXTENSION: Record<string, Category> = {};
const groups: [Category, string][] = [
  ["image", "png jpg jpeg gif webp bmp svg tif tiff heic heif ico avif raw cr2 nef"],
  ["video", "mp4 mkv mov avi webm m4v wmv flv mpg mpeg"],
  ["audio", "mp3 wav flac ogg m4a aac opus wma"],
  ["document", "pdf doc docx odt rtf ppt pptx odp epub pages key"],
  ["spreadsheet", "xls xlsx ods csv tsv numbers"],
  ["archive", "zip tar gz tgz bz2 xz 7z rar zst iso dmg"],
  ["code", "js ts tsx jsx rs py go java c h cpp hpp cs rb php sh ps1 bat html htm css scss vue svelte sql kt swift"],
  ["text", "txt md log json yaml yml toml xml ini conf cfg"],
];
for (const [category, exts] of groups) {
  for (const ext of exts.split(" ")) {
    BY_EXTENSION[ext] = category;
  }
}

/** The extension, lowercase, without the dot ("" for none or a dotfile). */
export function extensionOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 && i < name.length - 1 ? name.slice(i + 1).toLowerCase() : "";
}

export function categoryOf(e: Entry): Category | "folder" {
  return e.kind === "folder" ? "folder" : (BY_EXTENSION[extensionOf(e.name)] ?? "other");
}

/** What the Type column shows. */
export function typeLabel(e: Entry): string {
  if (e.kind === "folder") {
    return "Folder";
  }
  const ext = extensionOf(e.name);
  return ext === "" ? "File" : `${ext.toUpperCase()} file`;
}

/** `listed`: the provider's order (key order), folders first: what the
 * browser shows unless the user sorts. */
export const SORT_KEYS = ["listed", "name", "size", "modified", "type"] as const;
export type SortKey = (typeof SORT_KEYS)[number];
export type SortDirection = "ascending" | "descending";

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function compare(a: Entry, b: Entry, by: SortKey): number {
  switch (by) {
    case "listed":
      return 0;
    case "name":
      return collator.compare(a.name, b.name);
    case "size": {
      const sa = a.kind === "file" ? a.size : -1;
      const sb = b.kind === "file" ? b.size : -1;
      return sa - sb;
    }
    case "modified": {
      const ta = a.kind === "file" && a.lastModified !== null ? Date.parse(a.lastModified) : Number.NEGATIVE_INFINITY;
      const tb = b.kind === "file" && b.lastModified !== null ? Date.parse(b.lastModified) : Number.NEGATIVE_INFINITY;
      return ta === tb ? 0 : ta < tb ? -1 : 1;
    }
    case "type":
      return collator.compare(typeLabel(a), typeLabel(b));
  }
}

/** Folders first, then files; within each, by `by` (ties by name, then by
 * key, so the order is stable). A new array. */
export function sortEntries(entries: readonly Entry[], by: SortKey, direction: SortDirection): Entry[] {
  if (by === "listed") {
    const folders = entries.filter((e) => e.kind === "folder");
    const files = entries.filter((e) => e.kind === "file");
    return direction === "ascending" ? [...folders, ...files] : [...folders.reverse(), ...files.reverse()];
  }
  const sign = direction === "ascending" ? 1 : -1;
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) {
      return a.kind === "folder" ? -1 : 1;
    }
    const primary = compare(a, b, by) * sign;
    if (primary !== 0) {
      return primary;
    }
    const byName = collator.compare(a.name, b.name);
    if (byName !== 0) {
      return byName;
    }
    const ia = entryId(a);
    const ib = entryId(b);
    return ia < ib ? -1 : ia > ib ? 1 : 0;
  });
}

export interface Filter {
  show: "all" | "files" | "folders";
  /** Only these categories (files); empty: any. */
  categories: Category[];
  /** Only these extensions (lowercase, no dot); empty: any. */
  extensions: string[];
  /** Bytes; null: no bound. */
  minSize: number | null;
  maxSize: number | null;
  /** Epoch milliseconds; null: no bound. */
  modifiedFrom: number | null;
  modifiedTo: number | null;
}

export const NO_FILTER: Filter = {
  show: "all",
  categories: [],
  extensions: [],
  minSize: null,
  maxSize: null,
  modifiedFrom: null,
  modifiedTo: null,
};

export function isFiltering(f: Filter): boolean {
  return (
    f.show !== "all" ||
    f.categories.length > 0 ||
    f.extensions.length > 0 ||
    f.minSize !== null ||
    f.maxSize !== null ||
    f.modifiedFrom !== null ||
    f.modifiedTo !== null
  );
}

/** Whether `e` passes `f`. Folders have no size, type or date: any of those
 * bounds hides them. */
export function passes(e: Entry, f: Filter): boolean {
  if (f.show === "files" && e.kind !== "file") {
    return false;
  }
  if (f.show === "folders" && e.kind !== "folder") {
    return false;
  }
  const fileOnly =
    f.categories.length > 0 ||
    f.extensions.length > 0 ||
    f.minSize !== null ||
    f.maxSize !== null ||
    f.modifiedFrom !== null ||
    f.modifiedTo !== null;
  if (e.kind === "folder") {
    return !fileOnly;
  }
  const category = categoryOf(e);
  if (f.categories.length > 0 && (category === "folder" || !f.categories.includes(category))) {
    return false;
  }
  if (f.extensions.length > 0 && !f.extensions.includes(extensionOf(e.name))) {
    return false;
  }
  if (f.minSize !== null && e.size < f.minSize) {
    return false;
  }
  if (f.maxSize !== null && e.size > f.maxSize) {
    return false;
  }
  if (f.modifiedFrom !== null || f.modifiedTo !== null) {
    const t = e.lastModified === null ? Number.NaN : Date.parse(e.lastModified);
    if (Number.isNaN(t)) {
      return false;
    }
    if (f.modifiedFrom !== null && t < f.modifiedFrom) {
      return false;
    }
    if (f.modifiedTo !== null && t > f.modifiedTo) {
      return false;
    }
  }
  return true;
}

export function filterEntries(entries: readonly Entry[], f: Filter): Entry[] {
  return isFiltering(f) ? entries.filter((e) => passes(e, f)) : [...entries];
}

/** Extensions typed by the user ("pdf, .JPG docx") as a clean list. */
export function parseExtensions(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[\s,;]+/)
        .map((x) => x.replace(/^\.+/, "").toLowerCase())
        .filter((x) => x !== ""),
    ),
  ];
}

// --------------------------------------------------------------- selection

export interface Selection {
  ids: ReadonlySet<string>;
  /** Where a Shift-click range starts. */
  anchor: string | null;
}

export const EMPTY_SELECTION: Selection = { ids: new Set(), anchor: null };

export type SelectMode = "replace" | "toggle" | "range" | "add-range";

/** The selection after a click on `id` with `mode` (plain click: replace;
 * Ctrl or Cmd: toggle; Shift: range from the anchor; Ctrl+Shift: add the
 * range). `order` is the visible order. */
export function select(sel: Selection, id: string, mode: SelectMode, order: readonly string[]): Selection {
  switch (mode) {
    case "replace":
      return { ids: new Set([id]), anchor: id };
    case "toggle": {
      const ids = new Set(sel.ids);
      if (ids.has(id)) {
        ids.delete(id);
      } else {
        ids.add(id);
      }
      return { ids, anchor: id };
    }
    case "range":
    case "add-range": {
      const from = sel.anchor === null ? -1 : order.indexOf(sel.anchor);
      const to = order.indexOf(id);
      if (from < 0 || to < 0) {
        return { ids: new Set([id]), anchor: id };
      }
      const [lo, hi] = from <= to ? [from, to] : [to, from];
      const range = order.slice(lo, hi + 1);
      const ids = mode === "add-range" ? new Set([...sel.ids, ...range]) : new Set(range);
      return { ids, anchor: sel.anchor };
    }
  }
}

/** The mode a mouse click with these modifier keys means. */
export function modeOf(e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): SelectMode {
  const toggle = e.ctrlKey || e.metaKey;
  if (e.shiftKey) {
    return toggle ? "add-range" : "range";
  }
  return toggle ? "toggle" : "replace";
}

export function selectAll(order: readonly string[]): Selection {
  return { ids: new Set(order), anchor: order[0] ?? null };
}

/** Keeps only ids still present (after a refresh, a filter, an operation). */
export function pruneSelection(sel: Selection, present: readonly string[]): Selection {
  const keep = new Set(present);
  const ids = new Set([...sel.ids].filter((id) => keep.has(id)));
  return { ids, anchor: sel.anchor !== null && keep.has(sel.anchor) ? sel.anchor : null };
}

/** The id `step` rows away from `current` in `order` (arrow keys). */
export function moveFocus(order: readonly string[], current: string | null, step: number): string | null {
  if (order.length === 0) {
    return null;
  }
  const i = current === null ? -1 : order.indexOf(current);
  const next = i < 0 ? (step > 0 ? 0 : order.length - 1) : Math.min(order.length - 1, Math.max(0, i + step));
  return order[next] ?? null;
}

// ---------------------------------------------------------------- shortcuts

export const SHORTCUT_ACTIONS = [
  "select-all",
  "copy",
  "cut",
  "paste",
  "delete",
  "rename",
  "escape",
  "refresh",
  "open",
  "up",
  "down",
  "extend-up",
  "extend-down",
] as const;
export type ShortcutAction = (typeof SHORTCUT_ACTIONS)[number];

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  target: EventTarget | null;
}

/** Whether the key event comes from a place where the user types. */
export function typing(target: EventTarget | null): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement)) {
    return false;
  }
  if (target.isContentEditable) {
    return true;
  }
  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") {
    return true;
  }
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    return !["checkbox", "radio", "button", "submit", "reset"].includes(type);
  }
  return false;
}

/** The file-list action a key means, or null. Never while typing. */
export function shortcutOf(e: KeyLike): ShortcutAction | null {
  if (e.altKey) {
    return null;
  }
  if (e.key === "Escape") {
    return "escape";
  }
  if (typing(e.target)) {
    return null;
  }
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (mod && !e.shiftKey) {
    switch (key) {
      case "a":
        return "select-all";
      case "c":
        return "copy";
      case "x":
        return "cut";
      case "v":
        return "paste";
      case "r":
        return "refresh";
    }
    return null;
  }
  if (mod) {
    return null;
  }
  switch (key) {
    case "Delete":
      return "delete";
    case "F2":
      return "rename";
    case "F5":
      return "refresh";
    case "Enter":
      return "open";
    case "ArrowUp":
      return e.shiftKey ? "extend-up" : "up";
    case "ArrowDown":
      return e.shiftKey ? "extend-down" : "down";
  }
  return null;
}
