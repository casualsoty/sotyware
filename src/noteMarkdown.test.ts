// The notes overlay's Markdown subset and editing helpers (ADR-052):
// what a note's text becomes, and what the editor's keys do to it.
import { describe, expect, it, vi } from "vitest";
import {
  continueList,
  firstLine,
  fold,
  indentLines,
  insertLink,
  lineAt,
  lineRange,
  parseBlocks,
  parseInline,
  parseItem,
  plainText,
  searchNotes,
  setLineKind,
  taskProgress,
  toggleTask,
  toggleWrap,
  type Block,
  type Inline,
} from "./noteMarkdown";

// Two emojis, whatever pictures this checkout bundles (noteEmoji.ts).
vi.mock("./noteEmoji", async (original) => {
  const actual = await original<typeof import("./noteEmoji")>();
  const emojis = [
    { id: "guardian", name: "Guardian", src: "/guardian.png" },
    { id: "thief", name: "Thief", src: "/thief.png" },
  ];
  return { ...actual, EMOJI_ROWS: [emojis], EMOJI_BY_ID: new Map(emojis.map((e) => [e.id, e])) };
});

const text = (v: string): Inline => ({ t: "text", v });

describe("inline formatting", () => {
  it("reads bold, italic, struck, code and links", () => {
    expect(parseInline("**heal** first, *then* ~~run~~ `F5`")).toEqual([
      { t: "strong", c: [text("heal")] },
      text(" first, "),
      { t: "em", c: [text("then")] },
      text(" "),
      { t: "del", c: [text("run")] },
      text(" "),
      { t: "code", v: "F5" },
    ]);
    expect(parseInline("see [the map](https://example.com/map_(1)) or https://x.org/a.")).toEqual([
      text("see "),
      { t: "link", href: "https://example.com/map_(1)", c: [text("the map")] },
      text(" or "),
      { t: "link", href: "https://x.org/a", c: [text("https://x.org/a")] },
      text("."),
    ]);
    expect(parseInline("__strong__ and _em_")).toEqual([
      { t: "strong", c: [text("strong")] },
      text(" and "),
      { t: "em", c: [text("em")] },
    ]);
  });

  it("nests emphasis", () => {
    expect(parseInline("**bold *and italic* bold**")).toEqual([
      { t: "strong", c: [text("bold "), { t: "em", c: [text("and italic")] }, text(" bold")] },
    ]);
    expect(parseInline("*a **b** c*")).toEqual([{ t: "em", c: [text("a "), { t: "strong", c: [text("b")] }, text(" c")] }]);
  });

  it("leaves plain text that only looks like Markdown alone", () => {
    expect(parseInline("5 * 3 * 2")).toEqual([text("5 * 3 * 2")]);
    expect(parseInline("snake_case_name and file_name.txt")).toEqual([text("snake_case_name and file_name.txt")]);
    expect(parseInline("50% of 2*x")).toEqual([text("50% of 2*x")]);
    expect(parseInline("**not closed")).toEqual([text("**not closed")]);
    expect(parseInline("a ~ b ~ c")).toEqual([text("a ~ b ~ c")]);
    expect(parseInline("\\*literal\\*")).toEqual([text("*literal*")]);
    expect(parseInline("`unclosed")).toEqual([text("`unclosed")]);
  });

  it("makes links of web addresses only", () => {
    expect(parseInline("[x](javascript:alert(1))")).toEqual([text("[x](javascript:alert(1))")]);
    expect(parseInline("[x](file:///etc/passwd)")).toEqual([text("[x](file:///etc/passwd)")]);
    expect(parseInline("ftp://host/file")).toEqual([text("ftp://host/file")]);
    expect(parseInline("<https://a.example/b>")).toEqual([
      { t: "link", href: "https://a.example/b", c: [text("https://a.example/b")] },
    ]);
    // Not inside a word, and an address needs a host.
    expect(parseInline("xhttps://a.b")).toEqual([text("xhttps://a.b")]);
    expect(parseInline("https:// later")).toEqual([text("https:// later")]);
    // A link's own text holds no link.
    expect(parseInline("[https://a.example](https://b.example)")).toEqual([
      { t: "link", href: "https://b.example", c: [text("https://a.example")] },
    ]);
    // Code spans hide what is inside them.
    expect(parseInline("`**x** https://a.b`")).toEqual([{ t: "code", v: "**x** https://a.b" }]);
  });

  it("gives the plain text of formatted content", () => {
    expect(plainText(parseInline("**Phase 2** at [50%](https://w.example) `F5`"))).toBe("Phase 2 at 50% F5");
  });
});

describe("emojis", () => {
  const guardian = { t: "emoji", e: { id: "guardian", name: "Guardian", src: "/guardian.png" } } as const;
  const thief = { t: "emoji", e: { id: "thief", name: "Thief", src: "/thief.png" } } as const;

  it("reads a bundled emoji's shortcode, also next to another or in formatting", () => {
    expect(parseInline("Tank: :guardian: then :thief:!")).toEqual([text("Tank: "), guardian, text(" then "), thief, text("!")]);
    expect(parseInline(":guardian::thief:")).toEqual([guardian, thief]);
    expect(parseInline("**:thief:**")).toEqual([{ t: "strong", c: [thief] }]);
  });

  it("leaves unknown shortcodes, shortcodes inside words and code as text", () => {
    expect(parseInline(":ranger: a:thief: :Thief:")).toEqual([text(":ranger: a:thief: :Thief:")]);
    expect(parseInline("`:thief:`")).toEqual([{ t: "code", v: ":thief:" }]);
    expect(parseInline("\\:thief:")).toEqual([text(":thief:")]);
  });

  it("reads as the profession's name, which search finds", () => {
    expect(plainText(parseInline("Party: :guardian: :thief:"))).toBe("Party: Guardian Thief");
    expect(searchNotes([{ id: "a", title: "Party", body: "- :thief: stealth" }], "thief")).toHaveLength(1);
  });
});

describe("blocks", () => {
  it("reads the example note from the request", () => {
    const note = [
      "Boss strategy",
      "",
      "* Prepare healing items",
      "* Equip appropriate gear",
      "* Check boss weaknesses",
      "",
      "Reminders",
      "",
      "* Phase 2 begins at approximately 50%.",
      "* Prioritize the adds before continuing.",
    ].join("\n");
    const blocks = parseBlocks(note);
    expect(blocks.map((b) => b.t)).toEqual(["paragraph", "list", "paragraph", "list"]);
    const list = blocks[1] as Extract<Block, { t: "list" }>;
    expect(list.items.map((i) => plainText(i.content))).toEqual([
      "Prepare healing items",
      "Equip appropriate gear",
      "Check boss weaknesses",
    ]);
    expect(list.items.map((i) => i.line)).toEqual([2, 3, 4]);
    expect(list.ordered).toBe(false);
  });

  it("reads headings, checklists, numbers, quotes, code and rules", () => {
    const blocks = parseBlocks(
      [
        "# Raid night",
        "## Phase 2",
        "#### Small heading",
        "#hashtag stays text",
        "- [ ] Heal",
        "- [x] Gear",
        "3) Third",
        "4) Fourth",
        "> Tip: dodge left",
        "> then right",
        "```",
        "**not bold**",
        "```",
        "---",
        "after",
      ].join("\n"),
    );
    expect(blocks.map((b) => b.t)).toEqual([
      "heading",
      "heading",
      "heading",
      "paragraph",
      "list",
      "list",
      "quote",
      "code",
      "rule",
      "paragraph",
    ]);
    expect(blocks.slice(0, 3).map((b) => (b.t === "heading" ? b.level : 0))).toEqual([1, 2, 3]);
    const tasks = blocks[4] as Extract<Block, { t: "list" }>;
    expect(tasks.items.map((i) => i.task)).toEqual([false, true]);
    const numbered = blocks[5] as Extract<Block, { t: "list" }>;
    expect([numbered.ordered, numbered.start]).toEqual([true, 3]);
    expect(blocks[6]).toEqual({ t: "quote", content: [text("Tip: dodge left"), { t: "br" }, text("then right")], line: 8 });
    expect(blocks[7]).toEqual({ t: "code", text: "**not bold**", line: 10 });
    expect(blocks[8]).toEqual({ t: "rule", line: 13 });
  });

  it("keeps a paragraph's line breaks", () => {
    expect(parseBlocks("line one\nline two\n\nnext")).toEqual([
      { t: "paragraph", content: [text("line one"), { t: "br" }, text("line two")], line: 0 },
      { t: "paragraph", content: [text("next")], line: 3 },
    ]);
  });

  it("nests lists by indentation and continues items on indented lines", () => {
    const [list] = parseBlocks(["- Loot", "  - [ ] Sword", "  - [x] Shield", "    with spikes", "- Route", "1. not nested"].join("\n"));
    const l = list as Extract<Block, { t: "list" }>;
    expect(l.items.map((i) => plainText(i.content))).toEqual(["Loot", "Route"]);
    const nested = l.items[0]?.children[0] as Extract<Block, { t: "list" }>;
    expect(nested.items.map((i) => [plainText(i.content), i.task, i.line])).toEqual([
      ["Sword", false, 1],
      ["Shield with spikes", true, 2],
    ]);
    const blocks = parseBlocks(["- a", "1. b"].join("\n"));
    expect(blocks.map((b) => (b.t === "list" ? b.ordered : null))).toEqual([false, true]);
  });

  it("is never confused by Windows line ends or an empty note", () => {
    expect(parseBlocks("")).toEqual([]);
    expect(parseBlocks("\n\n  \n")).toEqual([]);
    expect(parseBlocks("a\r\nb")).toEqual([{ t: "paragraph", content: [text("a"), { t: "br" }, text("b")], line: 0 }]);
    // A rule is not a list of dashes; a lone marker is an empty item.
    expect(parseBlocks("- - -").map((b) => b.t)).toEqual(["rule"]);
    expect(parseItem("- ")?.text).toBe("");
    expect(parseItem("-x")).toBeNull();
  });
});

describe("checklists", () => {
  it("toggles the checkbox of one line only", () => {
    const body = "- [ ] Heal\n  * [x] Gear\n- plain\n1. [ ] numbered";
    expect(toggleTask(body, 0)).toBe("- [x] Heal\n  * [x] Gear\n- plain\n1. [ ] numbered");
    expect(toggleTask(body, 1)).toBe("- [ ] Heal\n  * [ ] Gear\n- plain\n1. [ ] numbered");
    expect(toggleTask(body, 3)).toBe("- [ ] Heal\n  * [x] Gear\n- plain\n1. [x] numbered");
    expect(toggleTask(body, 2)).toBe(body);
    expect(toggleTask(body, 9)).toBe(body);
    expect(toggleTask("- [X] Done", 0)).toBe("- [ ] Done");
  });

  it("counts the items and the ticked ones, not in code", () => {
    expect(taskProgress("- [ ] a\n- [x] b\n  - [X] c\n```\n- [ ] code\n```\nplain")).toEqual({ done: 2, total: 3 });
    expect(taskProgress("nothing")).toEqual({ done: 0, total: 0 });
  });
});

describe("editing keys", () => {
  it("continues a list on Enter", () => {
    expect(continueList("- [x] Heal", 10)).toEqual({ value: "- [x] Heal\n- [ ] ", start: 17, end: 17 });
    expect(continueList("  * Loot", 8)).toEqual({ value: "  * Loot\n  * ", start: 13, end: 13 });
    expect(continueList("9. Ninth", 8)).toEqual({ value: "9. Ninth\n10. ", start: 13, end: 13 });
    expect(continueList("1) a", 4)?.value).toBe("1) a\n2) ");
    // In the middle: the rest goes to the new item.
    expect(continueList("- left right", 7)).toEqual({ value: "- left\n- right", start: 9, end: 9 });
  });

  it("ends a list on Enter in an empty item, and does nothing elsewhere", () => {
    expect(continueList("- a\n- ", 6)).toEqual({ value: "- a\n", start: 4, end: 4 });
    expect(continueList("- a\n- [ ] ", 10)).toEqual({ value: "- a\n", start: 4, end: 4 });
    expect(continueList("plain text", 5)).toBeNull();
    // On the marker itself: a plain new line.
    expect(continueList("- item", 0)).toBeNull();
  });

  it("indents and outdents list lines only", () => {
    expect(indentLines("- a\n- b", 4, 4, false)).toEqual({ value: "- a\n  - b", start: 6, end: 6 });
    expect(indentLines("- a\n  - b", 6, 9, true)).toEqual({ value: "- a\n- b", start: 4, end: 7 });
    expect(indentLines("- a\n- b", 0, 7, false)?.value).toBe("  - a\n  - b");
    expect(indentLines("plain", 2, 2, false)).toBeNull();
  });

  it("wraps and unwraps the selection", () => {
    expect(toggleWrap("heal now", 0, 4, "**")).toEqual({ value: "**heal** now", start: 2, end: 6 });
    expect(toggleWrap("**heal** now", 2, 6, "**")).toEqual({ value: "heal now", start: 0, end: 4 });
    expect(toggleWrap("**heal** now", 0, 8, "**")).toEqual({ value: "heal now", start: 0, end: 4 });
    expect(toggleWrap("ab", 1, 1, "*")).toEqual({ value: "a**b", start: 2, end: 2 });
  });

  it("turns lines into headings, lists, checklists and quotes and back", () => {
    expect(setLineKind("Heal\nGear", 0, 9, "task").value).toBe("- [ ] Heal\n- [ ] Gear");
    expect(setLineKind("- [ ] Heal\n- [ ] Gear", 0, 21, "task").value).toBe("Heal\nGear");
    expect(setLineKind("- Heal\n- Gear", 0, 13, "number").value).toBe("1. Heal\n2. Gear");
    expect(setLineKind("Title", 0, 0, "heading").value).toBe("## Title");
    expect(setLineKind("## Title", 0, 0, "heading").value).toBe("Title");
    expect(setLineKind("  - [x] nested", 3, 3, "bullet").value).toBe("  - nested");
    expect(setLineKind("tip", 0, 3, "quote")).toEqual({ value: "> tip", start: 5, end: 5 });
  });

  it("inserts a link with its address selected", () => {
    expect(insertLink("see map", 4, 7)).toEqual({ value: "see [map](https://)", start: 10, end: 18 });
    expect(insertLink("", 0, 0).value).toBe("[link](https://)");
  });

  it("finds lines", () => {
    const v = "ab\ncd\n\nef";
    expect([lineAt(v, 0), lineAt(v, 3), lineAt(v, 6), lineAt(v, 99)]).toEqual([0, 1, 2, 3]);
    expect(lineRange(v, 1)).toEqual({ start: 3, end: 5 });
    expect(lineRange(v, 3)).toEqual({ start: 7, end: 9 });
    expect(lineRange(v, 9)).toEqual({ start: 9, end: 9 });
  });
});

describe("search", () => {
  const notes = [
    { id: "a", title: "Boss strategy", body: "- [ ] Prepare healing items\nPhase 2 begins at 50%." },
    { id: "b", title: "Route", body: "Bridge, then the cave. The boss is last." },
    { id: "c", title: "Élite loot", body: "Épée longue" },
  ];

  it("matches every word in titles and text, titles first, ignoring case and accents", () => {
    expect(searchNotes(notes, "boss").map((h) => h.id)).toEqual(["a", "b"]);
    expect(searchNotes(notes, "BOSS cave").map((h) => h.id)).toEqual(["b"]);
    expect(searchNotes(notes, "elite epee").map((h) => h.id)).toEqual(["c"]);
    expect(searchNotes(notes, "  ")).toEqual([]);
    expect(searchNotes(notes, "dragon")).toEqual([]);
  });

  it("shows the text around the match", () => {
    const [hit] = searchNotes(notes, "phase");
    expect(hit?.snippet).toBe("Prepare healing items Phase 2 begins at 50%.");
    const long = { id: "l", title: "Long", body: `${"word ".repeat(20)}needle ${"tail ".repeat(20)}` };
    const [far] = searchNotes([long], "needle");
    expect(far?.snippet.startsWith("…")).toBe(true);
    expect(far?.snippet.endsWith("…")).toBe(true);
    expect(far?.snippet).toContain("needle");
    // Checkboxes and marks are not text.
    expect(searchNotes(notes, "x")).toEqual([]);
    expect(fold("Épée")).toBe("epee");
  });

  it("names a note by its first line of text", () => {
    expect(firstLine("\n## **Phase** 2\nmore")).toBe("Phase 2");
    expect(firstLine("- [ ] heal")).toBe("heal");
    expect(firstLine("")).toBe("");
    expect(firstLine("x".repeat(100))).toHaveLength(80);
  });
});
