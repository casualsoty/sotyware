// The notes overlay's Markdown (ADR-052): a small, safe subset parsed into
// a tree that NoteView.svelte renders with Svelte's own elements (never
// as HTML, so a note can hold no script or remote content), and the
// editor's helpers (lists that continue on Enter, checkboxes that toggle,
// bold and italic around a selection). Pure functions over the text.
//
// What it reads: headings (`#` to `######`, shown in three sizes),
// paragraphs whose line breaks are kept (notes are written line by line),
// bulleted (`-`, `*`, `+`) and numbered (`1.`, `1)`) lists nested by
// indentation, checklists (`- [ ]`, `- [x]`), quotes (`>`), fenced code
// (```), rules (`---`), and inline **bold**, *italic*, ~~struck~~,
// `code`, [links](https://…) and bare https:// addresses. Anything else
// stays plain text, so a note written without Markdown reads as written.

// --------------------------------------------------------------- types

export type Inline =
  | { t: "text"; v: string }
  | { t: "strong"; c: Inline[] }
  | { t: "em"; c: Inline[] }
  | { t: "del"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "link"; href: string; c: Inline[] }
  | { t: "br" };

export interface ListItem {
  /** The source line of the item's marker (0-based). */
  line: number;
  /** `null`: not a checklist item; else whether it is checked. */
  task: boolean | null;
  content: Inline[];
  /** Nested lists. */
  children: Block[];
}

export type Block =
  | { t: "heading"; level: 1 | 2 | 3; content: Inline[]; line: number }
  | { t: "paragraph"; content: Inline[]; line: number }
  | { t: "list"; ordered: boolean; start: number; items: ListItem[]; line: number }
  | { t: "quote"; content: Inline[]; line: number }
  | { t: "code"; text: string; line: number }
  | { t: "rule"; line: number };

/** A change of the editor's text: the new text and selection. */
export interface Edit {
  value: string;
  start: number;
  end: number;
}

// ------------------------------------------------------------- inlines

const ESCAPABLE = new Set("\\`*_{}[]()#+-.!~>|".split(""));

function isSpace(c: string | undefined): boolean {
  return c === undefined || /\s/.test(c);
}

function isWordChar(c: string | undefined): boolean {
  return c !== undefined && /[\p{L}\p{N}]/u.test(c);
}

/** The length of the run of `ch` starting at `i`. */
function runLength(s: string, i: number, ch: string): number {
  let n = 0;
  while (s[i + n] === ch) {
    n += 1;
  }
  return n;
}

/** Where the closing `marker` of an emphasis opened before `from` is, or
 * -1. A closer follows a non-space character; an underscore closer is not
 * followed by a letter or digit; a single `*` or `_` closer is not part of
 * a longer run (so `*a **b** c*` closes at the last `*`). */
function findCloser(s: string, from: number, marker: string): number {
  const ch = marker[0] ?? "";
  for (let i = from; i < s.length; i += 1) {
    const c = s[i];
    if (c === "\\") {
      i += 1;
      continue;
    }
    if (c === "`") {
      const close = s.indexOf("`", i + 1);
      if (close > i) {
        i = close;
      }
      continue;
    }
    if (c !== ch) {
      continue;
    }
    const run = runLength(s, i, ch);
    if (marker.length === 1 && run > 1) {
      i += run - 1;
      continue;
    }
    if (run < marker.length) {
      continue;
    }
    if (i > from && !isSpace(s[i - 1]) && !(ch === "_" && isWordChar(s[i + marker.length]))) {
      return i;
    }
    i += run - 1;
  }
  return -1;
}

/** Trailing punctuation is not part of a bare address. */
function trimUrl(url: string): string {
  let out = url;
  for (;;) {
    const last = out.at(-1);
    if (last === undefined) {
      return out;
    }
    if (".,;:!?'\"".includes(last)) {
      out = out.slice(0, -1);
    } else if (last === ")" && (out.match(/\(/g)?.length ?? 0) < (out.match(/\)/g)?.length ?? 0)) {
      out = out.slice(0, -1);
    } else {
      return out;
    }
  }
}

function pushText(out: Inline[], v: string): void {
  if (v === "") {
    return;
  }
  const last = out.at(-1);
  if (last?.t === "text") {
    last.v += v;
  } else {
    out.push({ t: "text", v });
  }
}

/** The inline content of one line. `links`: whether links are read (not
 * inside a link's own text). */
export function parseInline(s: string, links = true): Inline[] {
  const out: Inline[] = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i] ?? "";
    // \* and the like: the character itself.
    if (c === "\\" && i + 1 < s.length && ESCAPABLE.has(s[i + 1] ?? "")) {
      pushText(out, s[i + 1] ?? "");
      i += 2;
      continue;
    }
    if (c === "`") {
      const run = runLength(s, i, "`");
      const fence = "`".repeat(run);
      const close = s.indexOf(fence, i + run);
      if (close > i) {
        let code = s.slice(i + run, close);
        if (code.length > 2 && code.startsWith(" ") && code.endsWith(" ")) {
          code = code.slice(1, -1);
        }
        out.push({ t: "code", v: code });
        i = close + run;
        continue;
      }
      pushText(out, fence);
      i += run;
      continue;
    }
    if (links && c === "[") {
      // An address may hold balanced parentheses (Wikipedia's do).
      const m = /^\[([^\]]+)\]\(<?(https?:\/\/(?:[^\s<>()]|\([^\s<>()]*\))+)>?(?:\s+"[^"]*")?\)/i.exec(s.slice(i));
      if (m) {
        out.push({ t: "link", href: m[2] ?? "", c: parseInline(m[1] ?? "", false) });
        i += m[0].length;
        continue;
      }
    }
    if (links && c === "<") {
      const m = /^<(https?:\/\/[^\s<>]+)>/i.exec(s.slice(i));
      if (m) {
        out.push({ t: "link", href: m[1] ?? "", c: [{ t: "text", v: m[1] ?? "" }] });
        i += m[0].length;
        continue;
      }
    }
    if (links && (c === "h" || c === "H") && !isWordChar(s[i - 1])) {
      const m = /^https?:\/\/[^\s<>]+/i.exec(s.slice(i));
      if (m) {
        const url = trimUrl(m[0]);
        if (/^https?:\/\/[^/\s]/i.test(url)) {
          out.push({ t: "link", href: url, c: [{ t: "text", v: url }] });
          i += url.length;
          continue;
        }
      }
    }
    if (c === "*" || c === "_" || c === "~") {
      const run = runLength(s, i, c);
      const marker = c === "~" ? (run >= 2 ? "~~" : "") : run >= 2 ? c + c : c;
      const opensWord = !isSpace(s[i + marker.length]);
      const intraword = c === "_" && isWordChar(s[i - 1]);
      if (marker !== "" && opensWord && !intraword) {
        const close = findCloser(s, i + marker.length, marker);
        if (close > i + marker.length) {
          const inner = parseInline(s.slice(i + marker.length, close), links);
          out.push(marker === "~~" ? { t: "del", c: inner } : marker.length === 2 ? { t: "strong", c: inner } : { t: "em", c: inner });
          i = close + marker.length;
          continue;
        }
      }
      pushText(out, c.repeat(run));
      i += run;
      continue;
    }
    pushText(out, c);
    i += 1;
  }
  return out;
}

/** The text of inline content without its formatting (for screen
 * readers, tab names and search). */
export function plainText(nodes: readonly Inline[]): string {
  return nodes
    .map((n) => {
      switch (n.t) {
        case "text":
        case "code":
          return n.v;
        case "br":
          return " ";
        default:
          return plainText(n.c);
      }
    })
    .join("");
}

// -------------------------------------------------------------- blocks

const FENCE = /^ {0,3}(`{3,}|~{3,})/;
const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const QUOTE = /^ {0,3}>[ \t]?(.*)$/;
const ITEM = /^([ \t]*)([-*+]|\d{1,9}[.)])(?:[ \t]+(.*))?$/;
const TASK = /^\[([ xX])\](?:[ \t]+(.*))?$/;

/** Leading spaces, a tab counting as four. */
function indentOf(s: string): number {
  let n = 0;
  for (const c of s) {
    if (c === " ") {
      n += 1;
    } else if (c === "\t") {
      n += 4 - (n % 4);
    } else {
      break;
    }
  }
  return n;
}

export interface ItemLine {
  indent: number;
  /** The marker's text: `-`, `*`, `+`, `3.` or `3)`. */
  marker: string;
  ordered: boolean;
  number: number;
  task: boolean | null;
  /** The text after the marker and the checkbox. */
  text: string;
  /** Where that text starts on the line. */
  textStart: number;
}

/** A list item line, or `null`. A rule (`---`) is not a list item. */
export function parseItem(line: string): ItemLine | null {
  if (RULE.test(line)) {
    return null;
  }
  const m = ITEM.exec(line);
  if (!m) {
    return null;
  }
  const lead = m[1] ?? "";
  const marker = m[2] ?? "";
  const rest = m[3] ?? "";
  const ordered = /\d/.test(marker);
  const t = TASK.exec(rest);
  const restStart = line.length - rest.length;
  if (t) {
    const text = t[2] ?? "";
    return {
      indent: indentOf(lead),
      marker,
      ordered,
      number: ordered ? Number.parseInt(marker, 10) : 0,
      task: t[1] !== " ",
      text,
      textStart: line.length - text.length,
    };
  }
  return {
    indent: indentOf(lead),
    marker,
    ordered,
    number: ordered ? Number.parseInt(marker, 10) : 0,
    task: null,
    text: rest,
    textStart: restStart,
  };
}

function isBlockStart(line: string): boolean {
  return FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line) || parseItem(line) !== null;
}

interface Level {
  indent: number;
  list: Extract<Block, { t: "list" }>;
}

/** Parses a note's text into blocks. Every block knows its first source
 * line, so a click on it can put the editor's cursor there. */
export function parseBlocks(body: string): Block[] {
  const lines = body.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i] ?? "";
    if (line.trim() === "") {
      i += 1;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const mark = fence[1] ?? "```";
      const start = i;
      const text: string[] = [];
      i += 1;
      while (i < lines.length && !(lines[i] ?? "").trimStart().startsWith(mark)) {
        text.push(lines[i] ?? "");
        i += 1;
      }
      i += 1;
      blocks.push({ t: "code", text: text.join("\n"), line: start });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      const level = Math.min(3, (heading[1] ?? "#").length) as 1 | 2 | 3;
      blocks.push({ t: "heading", level, content: parseInline(heading[2] ?? ""), line: i });
      i += 1;
      continue;
    }
    if (RULE.test(line)) {
      blocks.push({ t: "rule", line: i });
      i += 1;
      continue;
    }
    if (QUOTE.test(line)) {
      const start = i;
      const content: Inline[] = [];
      while (i < lines.length) {
        const q = QUOTE.exec(lines[i] ?? "");
        if (!q) {
          break;
        }
        if (content.length > 0) {
          content.push({ t: "br" });
        }
        content.push(...parseInline(q[1] ?? ""));
        i += 1;
      }
      blocks.push({ t: "quote", content, line: start });
      continue;
    }
    if (parseItem(line) !== null) {
      i = parseList(lines, i, blocks);
      continue;
    }
    // A paragraph: its lines up to a blank line or another kind of block,
    // each line break kept.
    const start = i;
    const content: Inline[] = [];
    while (i < lines.length) {
      const l = lines[i] ?? "";
      if (l.trim() === "" || (i > start && isBlockStart(l))) {
        break;
      }
      if (content.length > 0) {
        content.push({ t: "br" });
      }
      content.push(...parseInline(l.trim()));
      i += 1;
    }
    blocks.push({ t: "paragraph", content, line: start });
  }
  return blocks;
}

/** Reads list lines from `i` into `out`; returns the line after them. A
 * deeper item nests under the item before it; an indented plain line
 * continues the item before it. */
function parseList(lines: string[], i: number, out: Block[]): number {
  const stack: Level[] = [];
  while (i < lines.length) {
    const line = lines[i] ?? "";
    if (line.trim() === "") {
      break;
    }
    const item = parseItem(line);
    if (item === null) {
      const last = stack.at(-1)?.list.items.at(-1);
      if (last !== undefined && indentOf(line) >= 2 && !isBlockStart(line.trimStart())) {
        last.content.push({ t: "br" }, ...parseInline(line.trim()));
        i += 1;
        continue;
      }
      break;
    }
    while (stack.length > 0 && item.indent < (stack.at(-1)?.indent ?? 0)) {
      stack.pop();
    }
    let level = stack.at(-1);
    if (level !== undefined && item.indent > level.indent + 1) {
      // Deeper: a list inside the last item.
      const parent = level.list.items.at(-1);
      if (parent !== undefined) {
        const list: Extract<Block, { t: "list" }> = { t: "list", ordered: item.ordered, start: item.number, items: [], line: i };
        parent.children.push(list);
        level = { indent: item.indent, list };
        stack.push(level);
      }
    } else if (level !== undefined && level.list.ordered !== item.ordered) {
      // The same depth but another kind of list: a new list.
      const list: Extract<Block, { t: "list" }> = { t: "list", ordered: item.ordered, start: item.number, items: [], line: i };
      const holder = stack.length > 1 ? stack[stack.length - 2]?.list.items.at(-1)?.children : out;
      (holder ?? out).push(list);
      stack[stack.length - 1] = { indent: item.indent, list };
      level = stack.at(-1);
    }
    if (level === undefined) {
      const list: Extract<Block, { t: "list" }> = { t: "list", ordered: item.ordered, start: item.number, items: [], line: i };
      out.push(list);
      level = { indent: item.indent, list };
      stack.push(level);
    }
    level.list.items.push({ line: i, task: item.task, content: parseInline(item.text.trim()), children: [] });
    i += 1;
  }
  return i;
}

// ---------------------------------------------------------- checklists

/** Ticks or unticks the checklist item on `line`; the text unchanged when
 * that line is no checklist item. */
export function toggleTask(body: string, line: number): string {
  const lines = body.split("\n");
  const l = lines[line];
  if (l === undefined) {
    return body;
  }
  const item = parseItem(l);
  if (item === null || item.task === null) {
    return body;
  }
  const box = l.indexOf("[", l.indexOf(item.marker) + item.marker.length);
  lines[line] = l.slice(0, box + 1) + (item.task ? " " : "x") + l.slice(box + 2);
  return lines.join("\n");
}

/** How many checklist items the note has, and how many are ticked. */
export function taskProgress(body: string): { done: number; total: number } {
  let done = 0;
  let total = 0;
  let fenced = false;
  for (const l of body.split("\n")) {
    if (FENCE.test(l)) {
      fenced = !fenced;
      continue;
    }
    const item = fenced ? null : parseItem(l);
    if (item !== null && item.task !== null) {
      total += 1;
      if (item.task) {
        done += 1;
      }
    }
  }
  return { done, total };
}

// --------------------------------------------------------------- lines

/** The 0-based line `offset` is on. */
export function lineAt(value: string, offset: number): number {
  let n = 0;
  const end = Math.min(Math.max(offset, 0), value.length);
  for (let i = 0; i < end; i += 1) {
    if (value[i] === "\n") {
      n += 1;
    }
  }
  return n;
}

/** Where line `line` starts, and where it ends (before its line break). */
export function lineRange(value: string, line: number): { start: number; end: number } {
  let start = 0;
  for (let n = 0; n < line; n += 1) {
    const next = value.indexOf("\n", start);
    if (next < 0) {
      return { start: value.length, end: value.length };
    }
    start = next + 1;
  }
  const nl = value.indexOf("\n", start);
  return { start, end: nl < 0 ? value.length : nl };
}

// -------------------------------------------------------------- editing

/** Enter at `caret` (no selection) on a list item: the next item, of the
 * same kind (the next number, an empty checkbox); on an empty item, the
 * list ends instead (the marker goes). `null` off a list. */
export function continueList(value: string, caret: number): Edit | null {
  const { start, end } = lineRange(value, lineAt(value, caret));
  const line = value.slice(start, end);
  const item = parseItem(line);
  if (item === null || caret < start + item.textStart) {
    return null;
  }
  if (item.text.trim() === "") {
    // An empty item: Enter ends the list.
    const v = value.slice(0, start) + value.slice(end);
    return { value: v, start, end: start };
  }
  const lead = line.slice(0, line.length - line.trimStart().length);
  const marker = item.ordered ? `${item.number + 1}${item.marker.slice(-1)}` : item.marker;
  const box = item.task === null ? "" : "[ ] ";
  const insert = `\n${lead}${marker} ${box}`;
  // Spaces around a split go, never the marker's own.
  const keep = start + item.textStart;
  let left = value.slice(0, caret);
  while (left.length > keep && (left.endsWith(" ") || left.endsWith("\t"))) {
    left = left.slice(0, -1);
  }
  const v = left + insert + value.slice(caret).replace(/^[ \t]+/, "");
  const at = left.length + insert.length;
  return { value: v, start: at, end: at };
}

/** Tab or Shift+Tab on list lines: every selected list line moves two
 * spaces in or out. `null` when no selected line is a list item (Tab then
 * moves the focus, as in any field). */
export function indentLines(value: string, start: number, end: number, outdent: boolean): Edit | null {
  const first = lineAt(value, start);
  const last = lineAt(value, Math.max(start, end - (end > start && value[end - 1] === "\n" ? 1 : 0)));
  const lines = value.split("\n");
  let touched = false;
  let shiftStart = 0;
  let shiftTotal = 0;
  for (let n = first; n <= last; n += 1) {
    const l = lines[n] ?? "";
    if (parseItem(l) === null) {
      continue;
    }
    touched = true;
    let delta: number;
    if (outdent) {
      const remove = Math.min(2, l.length - l.trimStart().length);
      lines[n] = l.slice(remove);
      delta = -remove;
    } else {
      lines[n] = `  ${l}`;
      delta = 2;
    }
    if (n === first) {
      shiftStart = delta;
    }
    shiftTotal += delta;
  }
  if (!touched) {
    return null;
  }
  const v = lines.join("\n");
  const lineStart = lineRange(v, first).start;
  return {
    value: v,
    start: Math.max(lineStart, start + shiftStart),
    end: Math.max(lineStart, end + shiftTotal),
  };
}

/** Puts `marker` around the selection (bold `**`, italic `*`, struck
 * `~~`, code `` ` ``), or takes it away when it is there already. Without
 * a selection the cursor lands between the two markers. */
export function toggleWrap(value: string, start: number, end: number, marker: string): Edit {
  const n = marker.length;
  const before = value.slice(Math.max(0, start - n), start);
  const after = value.slice(end, end + n);
  if (before === marker && after === marker) {
    return {
      value: value.slice(0, start - n) + value.slice(start, end) + value.slice(end + n),
      start: start - n,
      end: end - n,
    };
  }
  const inner = value.slice(start, end);
  if (inner.length >= 2 * n && inner.startsWith(marker) && inner.endsWith(marker)) {
    return {
      value: value.slice(0, start) + inner.slice(n, -n) + value.slice(end),
      start,
      end: end - 2 * n,
    };
  }
  return {
    value: value.slice(0, start) + marker + inner + marker + value.slice(end),
    start: start + n,
    end: end + n,
  };
}

export type LineKind = "heading" | "bullet" | "number" | "task" | "quote";

const PREFIX = /^([ \t]*)(?:#{1,6}[ \t]+|>[ \t]?|(?:[-*+]|\d{1,9}[.)])[ \t]+(?:\[[ xX]\][ \t]+)?)?/;

/** Makes every selected line a heading, a bullet, a numbered item, a
 * checklist item or a quote; when they all are that already, plain text
 * again. */
export function setLineKind(value: string, start: number, end: number, kind: LineKind): Edit {
  const first = lineAt(value, start);
  const last = lineAt(value, end);
  const lines = value.split("\n");
  const is = (l: string): boolean => {
    const item = parseItem(l);
    switch (kind) {
      case "heading":
        return HEADING.test(l) && l.trim() !== "";
      case "quote":
        return QUOTE.test(l);
      case "bullet":
        return item !== null && !item.ordered && item.task === null;
      case "number":
        return item !== null && item.ordered;
      case "task":
        return item !== null && item.task !== null;
    }
  };
  const all = lines.slice(first, last + 1).every(is);
  let number = 1;
  for (let n = first; n <= last; n += 1) {
    const l = lines[n] ?? "";
    const m = PREFIX.exec(l);
    const lead = m?.[1] ?? "";
    const text = l.slice(m?.[0].length ?? 0);
    if (all) {
      lines[n] = lead + text;
      continue;
    }
    const prefix =
      kind === "heading" ? "## " : kind === "quote" ? "> " : kind === "bullet" ? "- " : kind === "task" ? "- [ ] " : `${number}. `;
    number += 1;
    lines[n] = lead + prefix + text;
  }
  const v = lines.join("\n");
  const range = lineRange(v, last);
  return { value: v, start: range.end, end: range.end };
}

/** A link around the selection (its text), the address selected to be
 * typed over. */
export function insertLink(value: string, start: number, end: number): Edit {
  const text = value.slice(start, end) || "link";
  const url = "https://";
  const insert = `[${text}](${url})`;
  const at = start + text.length + 3;
  return { value: value.slice(0, start) + insert + value.slice(end), start: at, end: at + url.length };
}

// -------------------------------------------------------------- search

/** Lower case, without accents: what a search compares. */
export function fold(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase();
}

export interface SearchHit {
  id: string;
  title: string;
  /** Text around the first match in the note, or its first line. */
  snippet: string;
  inTitle: boolean;
}

/** A note's text as read: without list markers, checkboxes, heading and
 * quote marks or inline formatting, its lines joined by spaces. */
export function readableText(body: string): string {
  return body
    .split("\n")
    .map((l) => {
      const item = parseItem(l);
      const text = item !== null ? item.text : l.replace(/^[ \t]*(#{1,6}[ \t]+|>[ \t]?)/, "");
      return plainText(parseInline(text.trim())).trim();
    })
    .filter((l) => l !== "")
    .join(" ");
}

/** The notes whose title or text holds every word of `query`, titles
 * first, in the order given. What is searched is the text as read, so a
 * search for "x" does not find every ticked checkbox. */
export function searchNotes(notes: readonly { id: string; title: string; body: string }[], query: string): SearchHit[] {
  const words = fold(query).split(/\s+/).filter((w) => w !== "");
  if (words.length === 0) {
    return [];
  }
  const hits: SearchHit[] = [];
  for (const n of notes) {
    const title = fold(n.title);
    const readable = readableText(n.body);
    const body = fold(readable);
    if (!words.every((w) => title.includes(w) || body.includes(w))) {
      continue;
    }
    const inTitle = words.every((w) => title.includes(w));
    const at = words.map((w) => body.indexOf(w)).find((x) => x >= 0) ?? -1;
    let snippet: string;
    if (at >= 0) {
      // Folding keeps the length for the scripts notes are written in, so
      // the offset fits the text closely enough for a snippet.
      const from = Math.max(0, at - 30);
      const to = at + 60;
      snippet = `${from > 0 ? "…" : ""}${readable.slice(from, to).trim()}${to < readable.length ? "…" : ""}`;
    } else {
      snippet = firstLine(n.body);
    }
    hits.push({ id: n.id, title: n.title, snippet, inTitle });
  }
  return hits.sort((a, b) => Number(b.inTitle) - Number(a.inTitle));
}

/** A note's first line with text, without its Markdown marks. */
export function firstLine(body: string): string {
  for (const l of body.split("\n")) {
    const item = parseItem(l);
    const text = item !== null ? item.text : l.replace(/^[ \t]*(#{1,6}[ \t]+|>[ \t]?)/, "");
    const plain = plainText(parseInline(text.trim())).trim();
    if (plain !== "") {
      return plain.length > 80 ? `${plain.slice(0, 79)}…` : plain;
    }
  }
  return "";
}
