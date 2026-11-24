<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import NoteView from "./NoteView.svelte";
  import { Autosave, type Recovery, type SaveState } from "./noteAutosave";
  import {
    continueList,
    indentLines,
    insertLink,
    lineAt,
    lineRange,
    parseItem,
    plainText,
    parseInline,
    searchNotes,
    setLineKind,
    taskProgress,
    toggleTask,
    toggleWrap,
    type Edit,
    type LineKind,
  } from "./noteMarkdown";
  import {
    BODY_MAX,
    createNote,
    deleteNote,
    duplicateNote,
    listNotes,
    moveNote,
    openNoteLink,
    openOverlaySettings,
    OverlayError,
    saveNote,
    selectNote,
    setNotePinned,
    setOverlayCollapsed,
    setOverlayMode,
    setOverlayOpacity,
    setOverlayVisible,
    TITLE_MAX,
    unwatchOverlay,
    watchOverlay,
    type FocusTarget,
    type Note,
    type NoteOrder,
    type OverlayStatus,
  } from "./notesOverlay";

  // The gaming notes overlay's own window (ADR-052,
  // docs/architecture/notes-overlay.md): a compact, always-on-top panel of
  // notes. Notes, the window and the shortcuts are Rust's; this page shows
  // the notes, edits them (autosaved) and drives the window through
  // commands. Passive, the window ignores the mouse and this page takes no
  // keyboard: it only shows the note.

  let status = $state<OverlayStatus | null>(null);
  let notes = $state<Note[]>([]);
  let activeId = $state<string | null>(null);
  let loaded = $state(false);
  let error = $state<string | null>(null);
  let dismissedNotice = $state<string | null>(null);
  let editing = $state(false);
  let searching = $state(false);
  let query = $state("");
  let menuFor = $state<string | null>(null);
  let opacityOpen = $state(false);
  let opacityDraft = $state<number | null>(null);
  let confirming = $state<Note | null>(null);
  let saveStates = $state<Record<string, SaveState>>({});
  let dropAt = $state<number | null>(null);

  let titleInput: HTMLInputElement | undefined = $state();
  let editor: HTMLTextAreaElement | undefined = $state();
  let editButton: HTMLButtonElement | undefined = $state();
  let searchInput: HTMLInputElement | undefined = $state();
  let tabStrip: HTMLDivElement | undefined = $state();

  const active = $derived(notes.find((n) => n.id === activeId) ?? null);
  const passive = $derived(status?.mode === "passive");
  const collapsed = $derived(status?.collapsed ?? false);
  const opacity = $derived(opacityDraft ?? status?.opacity ?? 88);
  const progress = $derived(active === null ? { done: 0, total: 0 } : taskProgress(active.body));
  const hits = $derived(searching && query.trim() !== "" ? searchNotes(notes, query) : []);
  const modeKeys = $derived(status?.hotkeys.find((h) => h.action === "toggle-mode" && h.registered)?.keys ?? null);
  const nextTask = $derived(active === null ? null : firstOpenTask(active.body));
  const shownNotice = $derived(status?.notice !== null && status?.notice !== dismissedNotice ? (status?.notice ?? null) : null);
  const saving = $derived(active === null ? "saved" : (saveStates[active.id] ?? "saved"));

  /** The formatting bar's buttons: what they do, their icon, their name. */
  const FORMATS = [
    ["heading", "heading", "Heading"],
    ["bold", "bold", "Bold (Ctrl+B)"],
    ["italic", "italic", "Italic (Ctrl+I)"],
    ["strike", "strike", "Strikethrough (Ctrl+Shift+X)"],
    ["bullet", "list-bullet", "Bulleted list"],
    ["number", "list-number", "Numbered list"],
    ["task", "checklist", "Checklist (Ctrl+Enter ticks an item)"],
    ["link", "link", "Link (Ctrl+K)"],
  ] as const;

  // What the status said last, to follow only what changed.
  let seen = { activeSeq: -1, notesSeq: -1, focusSeq: -1 };
  let following: Promise<void> = Promise.resolve();
  /** A tab was dragged: its click does not select it. */
  let dragged = false;

  function message(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  function firstOpenTask(body: string): string | null {
    for (const line of body.split("\n")) {
      const item = parseItem(line);
      if (item !== null && item.task === false && item.text.trim() !== "") {
        return plainText(parseInline(item.text.trim()));
      }
    }
    return null;
  }

  function titleOf(n: Note): string {
    return n.title.trim() === "" ? "Untitled" : n.title.trim();
  }

  // ---------------------------------------------------------- autosave

  const autosave = new Autosave({
    read: (id) => {
      const n = notes.find((x) => x.id === id);
      return n === undefined ? null : { title: n.title, body: n.body, revision: n.revision };
    },
    send: (id, title, body, revision) => saveNote(id, title, body, revision),
    saved: (id, revision) => {
      const n = notes.find((x) => x.id === id);
      if (n !== undefined) {
        n.revision = revision;
      }
    },
    failed: (id, e) => recover(id, e),
    onstate: (id, s) => {
      saveStates[id] = s;
    },
  });

  /** After a failed save: Rust's revision after a conflict (what the user
   * typed wins: this page is the notes' only editor), nothing for a note
   * deleted meanwhile, a message otherwise. */
  async function recover(id: string, e: unknown): Promise<Recovery> {
    if (e instanceof OverlayError && e.kind === "not-found") {
      return "drop";
    }
    if (e instanceof OverlayError && e.kind === "conflict") {
      try {
        const list = await listNotes();
        const theirs = list.notes.find((n) => n.id === id);
        const mine = notes.find((n) => n.id === id);
        if (theirs === undefined || mine === undefined) {
          return "drop";
        }
        mine.revision = theirs.revision;
        return "retry";
      } catch (err) {
        error = `Not saved: ${message(err)}`;
        return "later";
      }
    }
    error = `Not saved: ${message(e)}`;
    return e instanceof OverlayError && e.kind === "invalid-input" ? "wait" : "later";
  }

  function flushAll(): void {
    void autosave.flushAll();
  }

  // ------------------------------------------------------------ status

  function onStatus(s: OverlayStatus): void {
    if (status !== null && s.seq < status.seq) {
      return;
    }
    const before = status;
    status = s;
    if (before === null) {
      seen = { activeSeq: s.activeSeq, notesSeq: s.notesSeq, focusSeq: s.focusSeq };
    }
    if (s.mode === "passive" && before?.mode !== "passive") {
      enterPassive();
    }
    if (s.collapsed && !(before?.collapsed ?? false)) {
      closeEditor(false);
      searching = false;
    }
    if (before !== null) {
      following = following.then(() => follow(s));
    }
  }

  /** What Rust did on its own (a shortcut): new notes read, the note it
   * chose shown, the keyboard where it asked. One change at a time. */
  async function follow(s: OverlayStatus): Promise<void> {
    if (s.notesSeq !== seen.notesSeq) {
      seen.notesSeq = s.notesSeq;
      await reload();
    }
    if (s.activeSeq !== seen.activeSeq) {
      seen.activeSeq = s.activeSeq;
      if (s.activeNote !== null && notes.some((n) => n.id === s.activeNote)) {
        showNote(s.activeNote, false);
      }
    }
    if (s.focusSeq !== seen.focusSeq) {
      seen.focusSeq = s.focusSeq;
      await focusOn(s.focusTarget);
    }
  }

  async function focusOn(target: FocusTarget): Promise<void> {
    await tick();
    if (passive || collapsed) {
      return;
    }
    if (target === "title") {
      titleInput?.focus();
      titleInput?.select();
    } else {
      editButton?.focus();
    }
  }

  /** Reads the notes again, keeping text not saved yet. */
  async function reload(): Promise<void> {
    try {
      const list = await listNotes();
      notes = list.notes.map((n) => {
        const mine = notes.find((x) => x.id === n.id);
        return mine !== undefined && autosave.pending(n.id) ? { ...n, title: mine.title, body: mine.body } : n;
      });
      if (!notes.some((n) => n.id === activeId)) {
        activeId = list.activeNote;
      }
    } catch (e) {
      error = message(e);
    }
  }

  function enterPassive(): void {
    closeEditor(false);
    menuFor = null;
    opacityOpen = false;
    searching = false;
    confirming = null;
    // The window takes no keyboard now; nothing here may hold it.
    if (document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
  }

  // ------------------------------------------------------------- notes

  function showNote(id: string | null, tell = true): void {
    if (activeId !== null && activeId !== id) {
      void autosave.flush(activeId);
    }
    if (activeId !== id) {
      closeEditor(false);
    }
    menuFor = null;
    activeId = id;
    if (id !== null && tell) {
      void selectNote(id).catch((e: unknown) => {
        error = message(e);
      });
    }
    void tick().then(() => document.getElementById(`tab-${id ?? ""}`)?.scrollIntoView?.({ block: "nearest", inline: "nearest" }));
  }

  function step(by: number): void {
    if (notes.length === 0) {
      return;
    }
    const i = notes.findIndex((n) => n.id === activeId);
    const next = notes[(((i < 0 ? 0 : i) + by) % notes.length + notes.length) % notes.length];
    if (next !== undefined) {
      showNote(next.id);
    }
  }

  async function newNote(): Promise<void> {
    searching = false;
    try {
      const n = await createNote();
      notes = [...notes, n];
      showNote(n.id, false);
      await tick();
      titleInput?.focus();
      titleInput?.select();
    } catch (e) {
      error = message(e);
    }
  }

  function applyOrder(order: NoteOrder[]): void {
    const index = new Map(order.map((o, i) => [o.id, i]));
    for (const n of notes) {
      const o = order.find((x) => x.id === n.id);
      if (o !== undefined) {
        n.pinned = o.pinned;
        n.position = o.position;
      }
    }
    notes = [...notes].sort((a, b) => (index.get(a.id) ?? 0) - (index.get(b.id) ?? 0));
  }

  async function pin(n: Note): Promise<void> {
    menuFor = null;
    try {
      applyOrder(await setNotePinned(n.id, !n.pinned));
    } catch (e) {
      error = message(e);
    }
  }

  async function move(n: Note, index: number): Promise<void> {
    menuFor = null;
    try {
      applyOrder(await moveNote(n.id, Math.max(0, index)));
    } catch (e) {
      error = message(e);
    }
  }

  async function duplicate(n: Note): Promise<void> {
    menuFor = null;
    try {
      // The copy is made from the database: what was typed goes there first.
      await autosave.flush(n.id);
      const { note, order } = await duplicateNote(n.id);
      notes = [...notes, note];
      applyOrder(order);
      showNote(note.id, false);
    } catch (e) {
      error = message(e);
    }
  }

  async function remove(n: Note): Promise<void> {
    confirming = null;
    autosave.forget(n.id);
    try {
      const next = await deleteNote(n.id);
      notes = notes.filter((x) => x.id !== n.id);
      activeId = null;
      showNote(next, false);
    } catch (e) {
      error = message(e);
    }
  }

  function edited(): void {
    if (active !== null) {
      autosave.changed(active.id);
    }
  }

  function toggled(line: number): void {
    if (active === null || passive) {
      return;
    }
    active.body = toggleTask(active.body, line);
    edited();
    // A tick is worth keeping at once.
    void autosave.flush(active.id);
  }

  async function followLink(href: string): Promise<void> {
    if (active === null) {
      return;
    }
    try {
      await autosave.flush(active.id);
      await openNoteLink(active.id, href);
    } catch (e) {
      error = message(e);
    }
  }

  // ------------------------------------------------------------ editor

  async function startEditing(line: number | null): Promise<void> {
    if (active === null || passive) {
      return;
    }
    editing = true;
    await tick();
    if (editor === undefined) {
      return;
    }
    editor.focus();
    const at = line === null ? editor.value.length : lineRange(editor.value, line).end;
    editor.setSelectionRange(at, at);
  }

  function closeEditor(focusEdit: boolean): void {
    if (!editing) {
      return;
    }
    editing = false;
    if (activeId !== null) {
      void autosave.flush(activeId);
    }
    if (focusEdit) {
      void tick().then(() => editButton?.focus());
    }
  }

  /** Applies an edit through the browser's own typing where it can, so
   * Undo still works; the text is then the note's. */
  function applyEdit(edit: Edit): void {
    const t = editor;
    if (t === undefined || active === null) {
      return;
    }
    const old = t.value;
    let from = 0;
    while (from < old.length && from < edit.value.length && old[from] === edit.value[from]) {
      from += 1;
    }
    let tail = 0;
    while (
      tail < old.length - from &&
      tail < edit.value.length - from &&
      old[old.length - 1 - tail] === edit.value[edit.value.length - 1 - tail]
    ) {
      tail += 1;
    }
    t.setSelectionRange(from, old.length - tail);
    const insert = edit.value.slice(from, edit.value.length - tail);
    const typed = typeof document.execCommand === "function" && document.execCommand("insertText", false, insert);
    if (!typed || t.value !== edit.value) {
      t.value = edit.value;
    }
    t.setSelectionRange(edit.start, edit.end);
    active.body = t.value;
    edited();
  }

  function format(kind: "bold" | "italic" | "strike" | "code" | "link" | LineKind): void {
    const t = editor;
    if (t === undefined) {
      return;
    }
    const [s, e] = [t.selectionStart, t.selectionEnd];
    const marks: Record<string, string> = { bold: "**", italic: "*", strike: "~~", code: "`" };
    const mark = marks[kind];
    applyEdit(
      mark !== undefined ? toggleWrap(t.value, s, e, mark) : kind === "link" ? insertLink(t.value, s, e) : setLineKind(t.value, s, e, kind as LineKind),
    );
  }

  function onEditorKey(e: KeyboardEvent): void {
    const t = editor;
    if (t === undefined) {
      return;
    }
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    let edit: Edit | null = null;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeEditor(true);
      return;
    }
    if (e.key === "Enter" && mod) {
      const v = toggleTask(t.value, lineAt(t.value, t.selectionStart));
      edit = v === t.value ? null : { value: v, start: t.selectionStart, end: t.selectionEnd };
    } else if (e.key === "Enter" && !e.shiftKey && !e.altKey && t.selectionStart === t.selectionEnd) {
      edit = continueList(t.value, t.selectionStart);
    } else if (e.key === "Tab" && !mod && !e.altKey) {
      edit = indentLines(t.value, t.selectionStart, t.selectionEnd, e.shiftKey);
    } else if (mod && !e.altKey && !e.shiftKey && (key === "b" || key === "i" || key === "k")) {
      const kind = key === "b" ? "bold" : key === "i" ? "italic" : "link";
      e.preventDefault();
      format(kind);
      return;
    } else if (mod && e.shiftKey && !e.altKey && key === "x") {
      e.preventDefault();
      format("strike");
      return;
    }
    if (edit !== null) {
      e.preventDefault();
      applyEdit(edit);
    }
  }

  function onTitleKey(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      e.preventDefault();
      void startEditing(null);
    }
  }

  // ------------------------------------------------------------ window

  async function setMode(mode: "interactive" | "passive"): Promise<void> {
    flushAll();
    try {
      status = await setOverlayMode(mode);
    } catch (e) {
      error = message(e);
    }
  }

  async function toggleCollapsed(): Promise<void> {
    flushAll();
    try {
      status = await setOverlayCollapsed(!collapsed);
    } catch (e) {
      error = message(e);
    }
  }

  async function hide(): Promise<void> {
    flushAll();
    try {
      await setOverlayVisible(false);
    } catch (e) {
      error = message(e);
    }
  }

  async function settings(): Promise<void> {
    flushAll();
    try {
      await openOverlaySettings();
    } catch (e) {
      error = message(e);
    }
  }

  async function commitOpacity(): Promise<void> {
    if (opacityDraft === null) {
      return;
    }
    try {
      status = await setOverlayOpacity(opacityDraft);
    } catch (e) {
      error = message(e);
    } finally {
      opacityDraft = null;
    }
  }

  /** The title bar moves the window; a double click collapses it. */
  function onBarDown(e: MouseEvent): void {
    if (e.button !== 0 || passive || (e.target instanceof Element && e.target.closest("button, input"))) {
      return;
    }
    if (e.detail === 2) {
      void toggleCollapsed();
      return;
    }
    void getCurrentWindow()
      .startDragging()
      .catch(() => undefined);
  }

  function onGripDown(e: MouseEvent): void {
    if (e.button === 0) {
      e.preventDefault();
      void getCurrentWindow()
        .startResizeDragging("SouthEast")
        .catch(() => undefined);
    }
  }

  // -------------------------------------------------------------- tabs

  function onTabKey(e: KeyboardEvent, index: number): void {
    const n = notes[index];
    if (n === undefined) {
      return;
    }
    if (e.altKey && e.shiftKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      void move(n, index + (e.key === "ArrowLeft" ? -1 : 1));
      return;
    }
    const to =
      e.key === "ArrowRight" ? index + 1 : e.key === "ArrowLeft" ? index - 1 : e.key === "Home" ? 0 : e.key === "End" ? notes.length - 1 : null;
    if (to === null || e.altKey || e.ctrlKey || e.metaKey) {
      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
        e.preventDefault();
        menuFor = n.id;
      }
      return;
    }
    e.preventDefault();
    const next = notes[(to + notes.length) % notes.length];
    if (next !== undefined) {
      showNote(next.id);
      void tick().then(() => document.getElementById(`tab-${next.id}`)?.focus());
    }
  }

  /** Dragging a tab sideways reorders it (pinned notes stay first). */
  function onTabPointerDown(e: PointerEvent, n: Note): void {
    if (e.button !== 0 || tabStrip === undefined) {
      return;
    }
    const strip = tabStrip;
    const startX = e.clientX;
    const tab = e.currentTarget as HTMLElement;
    let moving = false;
    dragged = false;
    const targetIndex = (x: number): number => {
      const tabs = [...strip.querySelectorAll<HTMLElement>("[role='tab']")];
      const i = tabs.findIndex((t) => x < t.getBoundingClientRect().left + t.getBoundingClientRect().width / 2);
      return i < 0 ? tabs.length : i;
    };
    const onMove = (ev: PointerEvent): void => {
      if (!moving && Math.abs(ev.clientX - startX) > 6) {
        moving = true;
        dragged = true;
        tab.setPointerCapture?.(ev.pointerId);
      }
      if (moving) {
        dropAt = targetIndex(ev.clientX);
      }
    };
    const onUp = (ev: PointerEvent): void => {
      tab.removeEventListener("pointermove", onMove);
      tab.removeEventListener("pointerup", onUp);
      tab.removeEventListener("pointercancel", onUp);
      if (moving) {
        const from = notes.findIndex((x) => x.id === n.id);
        const to = targetIndex(ev.clientX);
        dropAt = null;
        // Moving past itself: the index it ends at.
        void move(n, to > from ? to - 1 : to);
      }
    };
    tab.addEventListener("pointermove", onMove);
    tab.addEventListener("pointerup", onUp);
    tab.addEventListener("pointercancel", onUp);
  }

  function onTabClick(n: Note): void {
    if (dragged) {
      dragged = false;
      return;
    }
    showNote(n.id);
  }

  // -------------------------------------------------------------- search

  async function openSearch(): Promise<void> {
    searching = true;
    menuFor = null;
    await tick();
    searchInput?.focus();
    searchInput?.select();
  }

  function closeSearch(): void {
    searching = false;
    query = "";
  }

  function pick(id: string): void {
    closeSearch();
    showNote(id);
  }

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeSearch();
    } else if (e.key === "Enter") {
      const first = hits[0];
      if (first !== undefined) {
        e.preventDefault();
        pick(first.id);
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      document.querySelector<HTMLElement>(".hits button")?.focus();
    }
  }

  // ------------------------------------------------------------- keys

  function onWindowKey(e: KeyboardEvent): void {
    if (passive || collapsed || confirming !== null) {
      return;
    }
    if (e.key === "Escape") {
      if (menuFor !== null || opacityOpen || searching) {
        e.preventDefault();
        menuFor = null;
        opacityOpen = false;
        closeSearch();
      }
      return;
    }
    if (!(e.ctrlKey || e.metaKey) || e.altKey) {
      return;
    }
    const key = e.key.toLowerCase();
    if (key === "n" && !e.shiftKey) {
      e.preventDefault();
      void newNote();
    } else if (key === "f" && !e.shiftKey) {
      e.preventDefault();
      void openSearch();
    } else if (e.key === "Tab" || e.key === "PageDown" || e.key === "PageUp") {
      e.preventDefault();
      step(e.key === "PageUp" || (e.key === "Tab" && e.shiftKey) ? -1 : 1);
    } else if (/^[1-9]$/.test(e.key) && !e.shiftKey) {
      const n = notes[Number(e.key) - 1];
      if (n !== undefined) {
        e.preventDefault();
        showNote(n.id);
      }
    }
  }

  function onWindowPointer(e: PointerEvent): void {
    if (!(e.target instanceof Element)) {
      return;
    }
    if (menuFor !== null && !e.target.closest(".menu, .menu-button")) {
      menuFor = null;
    }
    if (opacityOpen && !e.target.closest(".opacity")) {
      opacityOpen = false;
    }
  }

  onMount(() => {
    let subscription: number | null = null;
    let stopped = false;
    void (async () => {
      try {
        const list = await listNotes();
        notes = list.notes;
        activeId = list.activeNote;
      } catch (e) {
        error = message(e);
      }
      loaded = true;
      try {
        const id = await watchOverlay(onStatus);
        if (stopped) {
          void unwatchOverlay(id).catch(() => undefined);
        } else {
          subscription = id;
        }
      } catch (e) {
        error = message(e);
      }
    })();
    const hidden = (): void => {
      if (document.hidden) {
        flushAll();
      }
    };
    window.addEventListener("blur", flushAll);
    window.addEventListener("pagehide", flushAll);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      stopped = true;
      if (subscription !== null) {
        void unwatchOverlay(subscription).catch(() => undefined);
      }
      window.removeEventListener("blur", flushAll);
      window.removeEventListener("pagehide", flushAll);
      document.removeEventListener("visibilitychange", hidden);
      flushAll();
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} onpointerdown={onWindowPointer} />

<div
  class="overlay"
  class:passive
  class:collapsed
  class:see-through={opacity < 75}
  data-size={status?.textSize ?? "medium"}
  data-mode={status?.mode ?? "interactive"}
  style="--panel-alpha: {opacity / 100}"
>
  <!-- The title bar: drag to move, double-click to collapse (the buttons
       do the same from the keyboard). -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <header class="bar" onmousedown={onBarDown}>
    <span class="bar-icon" aria-hidden="true"><Icon name="note" size={14} /></span>
    <div class="bar-title">
      <h1 class="note-name" title={active === null ? "" : titleOf(active)}>{active === null ? "Notes" : titleOf(active)}</h1>
      <!-- Paragraphs: WebKitGTK leaves a flex item span's text out of the
           accessibility tree (frontend.md). -->
      {#if collapsed && nextTask !== null}
        <p class="next" title="Next unchecked item">{nextTask}</p>
      {:else if progress.total > 0}
        <p class="progress" title="{progress.done} of {progress.total} checked">{progress.done}/{progress.total}</p>
      {/if}
    </div>
    {#if passive}
      <div class="passive-badge" role="status">
        <Icon name="passive" size={12} />
        <p>{modeKeys === null ? "Click-through · switch back from sotyware" : `Click-through · ${modeKeys}`}</p>
      </div>
    {:else}
      <div class="bar-actions">
        <button
          type="button"
          class="ghost icon small"
          aria-label="Passive: let clicks through to the game"
          title="Passive: let clicks through{modeKeys === null ? '' : ` (${modeKeys} switches back)`}"
          onclick={() => void setMode("passive")}><Icon name="passive" size={14} /></button
        >
        <div class="opacity">
          <button
            type="button"
            class="ghost icon small"
            aria-label="Background opacity, {opacity}%"
            aria-expanded={opacityOpen}
            title="Background opacity"
            onclick={() => (opacityOpen = !opacityOpen)}><Icon name="opacity" size={14} /></button
          >
          {#if opacityOpen}
            <div class="popover" role="group" aria-label="Background opacity">
              <input
                type="range"
                min="20"
                max="100"
                step="5"
                value={opacity}
                aria-label="Background opacity"
                aria-valuetext="{opacity}%"
                oninput={(e) => (opacityDraft = Number((e.currentTarget as HTMLInputElement).value))}
                onchange={() => void commitOpacity()}
              />
              <span class="value">{opacity}%</span>
            </div>
          {/if}
        </div>
        <button
          type="button"
          class="ghost icon small"
          aria-label={collapsed ? "Expand" : "Collapse to the title bar"}
          title={collapsed ? "Expand" : "Collapse"}
          onclick={() => void toggleCollapsed()}><Icon name={collapsed ? "chevron-down" : "chevron-up"} size={14} /></button
        >
        <button type="button" class="ghost icon small" aria-label="Overlay settings" title="Overlay settings" onclick={() => void settings()}
          ><Icon name="settings" size={14} /></button
        >
        <button
          type="button"
          class="ghost icon small close"
          aria-label="Hide the overlay (notes are kept)"
          title="Hide (notes are kept)"
          onclick={() => void hide()}><Icon name="close" size={14} /></button
        >
      </div>
    {/if}
  </header>

  {#if !collapsed}
    {#if error !== null || shownNotice !== null || status?.storageError}
      <div class="alert" role="alert">
        <p>{error ?? shownNotice ?? `The notes cannot be saved: ${status?.storageError}`}</p>
        {#if !passive}
          <button
            type="button"
            class="ghost icon small"
            aria-label="Dismiss"
            onclick={() => {
              error = null;
              dismissedNotice = status?.notice ?? null;
            }}><Icon name="close" size={12} /></button
          >
        {/if}
      </div>
    {/if}

    {#if !passive}
      <nav class="strip" aria-label="Notes">
        {#if searching}
          <input
            bind:this={searchInput}
            bind:value={query}
            type="search"
            class="search-field"
            placeholder="Search notes"
            aria-label="Search notes"
            onkeydown={onSearchKey}
          />
          <button type="button" class="ghost icon small" aria-label="Close the search" onclick={closeSearch}><Icon name="close" size={14} /></button>
        {:else}
          <div class="tabs" role="tablist" aria-label="Notes" bind:this={tabStrip}>
            {#each notes as n, i (n.id)}
              <button
                type="button"
                role="tab"
                id="tab-{n.id}"
                class="tab"
                class:pinned={n.pinned}
                class:drop-before={dropAt === i}
                class:drop-after={dropAt === notes.length && i === notes.length - 1}
                aria-selected={n.id === activeId}
                aria-controls={n.id === activeId ? "note-panel" : undefined}
                tabindex={n.id === activeId ? 0 : -1}
                title={titleOf(n)}
                onclick={() => onTabClick(n)}
                onkeydown={(e) => onTabKey(e, i)}
                onpointerdown={(e) => onTabPointerDown(e, n)}
                ondblclick={() => titleInput?.focus()}
                oncontextmenu={(e) => {
                  e.preventDefault();
                  showNote(n.id);
                  menuFor = n.id;
                }}
                >{#if n.pinned}<span class="pin" aria-hidden="true"><Icon name="pin" size={11} /></span><span class="visually-hidden">{"Pinned: "}</span>{/if}<span
                  class="tab-title">{titleOf(n)}</span
                ></button
              >
            {/each}
          </div>
          <button type="button" class="ghost icon small" aria-label="Search notes (Ctrl+F)" title="Search (Ctrl+F)" onclick={() => void openSearch()}
            ><Icon name="search" size={14} /></button
          >
          <button type="button" class="ghost icon small" aria-label="New note (Ctrl+N)" title="New note (Ctrl+N)" onclick={() => void newNote()}
            ><Icon name="plus" size={14} /></button
          >
        {/if}
      </nav>
    {/if}

    <div class="content" id="note-panel" role={passive ? undefined : "tabpanel"} aria-labelledby={active === null || passive ? undefined : `tab-${active.id}`}>
      {#if searching && query.trim() !== ""}
        <ul class="hits" aria-label="Search results">
          {#each hits as h (h.id)}
            <li>
              <button type="button" onclick={() => pick(h.id)}>
                <span class="hit-title">{h.title.trim() === "" ? "Untitled" : h.title}</span>
                {#if h.snippet !== ""}<span class="hit-snippet">{h.snippet}</span>{/if}
              </button>
            </li>
          {:else}
            <li class="none">No note matches “{query.trim()}”.</li>
          {/each}
        </ul>
      {:else if !loaded}
        <p class="muted-line" role="status">Loading…</p>
      {:else if active === null}
        <div class="no-notes">
          <p>No notes yet.</p>
          {#if !passive}
            <button type="button" class="primary small" onclick={() => void newNote()}><Icon name="plus" size={14} />New note</button>
            <p class="hint-line">Checklists, routes, boss strategies: anything you want to see while you play.</p>
          {/if}
        </div>
      {:else}
        {#if !passive}
          <div class="note-head">
            <input
              bind:this={titleInput}
              class="note-title"
              type="text"
              value={active.title}
              maxlength={TITLE_MAX}
              placeholder="Untitled"
              aria-label="Note title"
              oninput={(e) => {
                if (active !== null) {
                  active.title = (e.currentTarget as HTMLInputElement).value;
                  edited();
                }
              }}
              onkeydown={onTitleKey}
            />
            {#if !editing}
              <button bind:this={editButton} type="button" class="ghost icon small" aria-label="Edit the note" title="Edit (or click the text)" onclick={() => void startEditing(null)}
                ><Icon name="edit" size={14} /></button
              >
            {/if}
            <div class="menu-wrap">
              <button
                type="button"
                class="ghost icon small menu-button"
                aria-label="More actions for this note"
                aria-haspopup="menu"
                aria-expanded={menuFor === active.id}
                title="More"
                onclick={() => (menuFor = menuFor === active?.id ? null : (active?.id ?? null))}><Icon name="more" size={14} /></button
              >
              {#if menuFor === active.id}
                {@const n = active}
                {@const index = notes.findIndex((x) => x.id === n.id)}
                <div class="menu" role="menu" aria-label="Note actions">
                  <button type="button" role="menuitem" onclick={() => void pin(n)}><Icon name="pin" size={13} />{n.pinned ? "Unpin" : "Pin to the front"}</button>
                  <button type="button" role="menuitem" onclick={() => void duplicate(n)}><Icon name="copy" size={13} />Duplicate</button>
                  <button type="button" role="menuitem" disabled={index <= 0} onclick={() => void move(n, index - 1)}
                    ><Icon name="arrow-left" size={13} />Move left</button
                  >
                  <button type="button" role="menuitem" disabled={index >= notes.length - 1} onclick={() => void move(n, index + 1)}
                    ><Icon name="move" size={13} />Move right</button
                  >
                  <button
                    type="button"
                    role="menuitem"
                    class="danger"
                    onclick={() => {
                      menuFor = null;
                      confirming = n;
                    }}><Icon name="trash" size={13} />Delete…</button
                  >
                </div>
              {/if}
            </div>
          </div>
        {/if}
        <div class="note-body">
          {#if editing && !passive}
            <textarea
              bind:this={editor}
              class="editor"
              value={active.body}
              maxlength={BODY_MAX}
              aria-label="Note text (Markdown)"
              spellcheck="true"
              oninput={(e) => {
                if (active !== null) {
                  active.body = (e.currentTarget as HTMLTextAreaElement).value;
                  edited();
                }
              }}
              onkeydown={onEditorKey}
            ></textarea>
          {:else}
            <NoteView
              body={active.body}
              interactive={!passive}
              ontoggle={toggled}
              onedit={(line) => void startEditing(line)}
              onlink={(href) => void followLink(href)}
            />
          {/if}
        </div>
        {#if editing && !passive}
          <!-- Buttons keep the editor's focus and selection (mousedown). -->
          <div class="format" role="toolbar" aria-label="Formatting">
            {#each FORMATS as [kind, icon, label] (kind)}
              <button type="button" class="ghost icon small" aria-label={label} title={label} onmousedown={(e) => e.preventDefault()} onclick={() => format(kind)}
                ><Icon name={icon} size={14} /></button
              >
            {/each}
            <p class="save-state" role="status">{saving === "failed" ? "Not saved" : saving === "saved" ? "Saved" : "Saving…"}</p>
            <button type="button" class="small" onclick={() => closeEditor(true)}>Done</button>
          </div>
        {/if}
      {/if}
    </div>

    {#if !passive}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <span class="grip" title="Resize" aria-hidden="true" onmousedown={onGripDown}><Icon name="grip" size={12} /></span>
    {/if}
  {/if}
</div>

{#if confirming !== null}
  {@const n = confirming}
  <Modal title="Delete this note?" onclose={() => (confirming = null)}>
    <p>“{titleOf(n)}” and its text are deleted. This cannot be undone.</p>
    {#snippet footer()}
      <button type="button" onclick={() => (confirming = null)}>Cancel</button>
      <button type="button" class="primary danger" data-autofocus onclick={() => void remove(n)}>Delete</button>
    {/snippet}
  </Modal>
{/if}

<style>
  /* The overlay's page: no background of its own, the window is
     transparent and the panel below paints itself. */
  :global(html.overlay-page),
  :global(html.overlay-page body) {
    background: transparent;
    overflow: hidden;
  }

  .overlay {
    --ov-text: #ecebe7;
    --ov-muted: #b9b6af;
    --ov-subtle: #8f8c85;
    --ov-accent: #8fb2f7;
    --ov-border: rgb(255 255 255 / 11%);
    --ov-hover: rgb(255 255 255 / 9%);
    --ov-active: rgb(255 255 255 / 15%);
    --ov-panel: 22 21 20;
    --note-size: 14px;

    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    color: var(--ov-text);
    font-size: var(--note-size);
    background: rgb(var(--ov-panel) / var(--panel-alpha));
    border: 1px solid var(--ov-border);
    border-radius: 10px;
    transition:
      background-color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .overlay[data-size="small"] {
    --note-size: 13px;
  }

  .overlay[data-size="large"] {
    --note-size: 16px;
  }

  /* Over a bright game a see-through panel keeps its text readable. */
  .overlay.see-through {
    text-shadow:
      0 1px 2px rgb(0 0 0 / 85%),
      0 0 1px rgb(0 0 0 / 75%);
  }

  /* Passive: clicks go through (the window ignores the mouse); this is
     only the look of it, a dashed edge. */
  .overlay.passive {
    border-style: dashed;
    border-color: rgb(143 178 247 / 45%);
    pointer-events: none;
    user-select: none;
  }

  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 36px;
    padding: 0 var(--space-1) 0 var(--space-3);
    background: rgb(0 0 0 / calc(var(--panel-alpha) * 0.28));
    border-bottom: 1px solid var(--ov-border);
    cursor: default;
    user-select: none;
  }

  .collapsed .bar {
    border-bottom: none;
    background: transparent;
  }

  .bar-icon {
    display: inline-flex;
    color: var(--ov-accent);
  }

  .bar-title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
  }

  .note-name {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    font-size: 0.93em;
    font-weight: var(--weight-semibold);
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .progress,
  .next {
    flex: none;
    margin: 0;
    color: var(--ov-muted);
    font-size: 0.82em;
    font-variant-numeric: tabular-nums;
  }

  .next {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .next::before {
    content: "→ ";
    color: var(--ov-accent);
  }

  .bar-actions {
    display: flex;
    align-items: center;
    gap: 1px;
  }

  .overlay :global(button.ghost) {
    color: var(--ov-muted);
  }

  .overlay :global(button.ghost:hover:not(:disabled)) {
    background: var(--ov-hover);
    color: var(--ov-text);
  }

  .overlay :global(button.ghost:active:not(:disabled)) {
    background: var(--ov-active);
  }

  .close:hover:not(:disabled) {
    background: rgb(196 43 28 / 85%) !important;
    color: #fff !important;
  }

  .passive-badge p {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .passive-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    margin-right: var(--space-2);
    padding: 2px var(--space-2);
    overflow: hidden;
    border: 1px solid rgb(143 178 247 / 35%);
    border-radius: var(--radius-full);
    color: var(--ov-accent);
    font-size: 0.75em;
    white-space: nowrap;
    text-overflow: ellipsis;
    opacity: 0.85;
  }

  .opacity,
  .menu-wrap {
    position: relative;
  }

  .popover,
  .menu {
    position: absolute;
    z-index: var(--z-menu);
    top: calc(100% + 4px);
    right: 0;
    background: #262523;
    border: 1px solid var(--ov-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-md);
    text-shadow: none;
  }

  .popover {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
  }

  .popover input {
    width: 8.5rem;
    accent-color: var(--ov-accent);
  }

  .popover .value {
    min-width: 2.8em;
    color: var(--ov-muted);
    font-size: 0.82em;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .menu {
    display: flex;
    flex-direction: column;
    min-width: 11rem;
    padding: var(--space-1);
  }

  .menu button {
    justify-content: flex-start;
    gap: var(--space-2);
    min-height: 28px;
    padding: 0 var(--space-2);
    background: transparent;
    border-color: transparent;
    color: var(--ov-text);
    font-weight: var(--weight-regular);
  }

  .menu button:hover:not(:disabled) {
    background: var(--ov-hover);
  }

  .menu button.danger {
    color: #f08a80;
  }

  .alert {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-3);
    background: rgb(196 67 58 / 30%);
    border-bottom: 1px solid rgb(240 138 128 / 35%);
    font-size: 0.82em;
  }

  .alert p {
    flex: 1;
    min-width: 0;
    margin: 0;
  }

  .strip {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 32px;
    padding: 0 var(--space-1);
    border-bottom: 1px solid var(--ov-border);
  }

  .tabs {
    display: flex;
    flex: 1;
    gap: 2px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tabs::-webkit-scrollbar {
    display: none;
  }

  .tab {
    flex: none;
    gap: 4px;
    max-width: 11rem;
    min-height: 24px;
    padding: 0 var(--space-2);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--ov-muted);
    font-size: 0.82em;
    font-weight: var(--weight-medium);
    touch-action: none;
  }

  .tab:hover:not(:disabled) {
    background: var(--ov-hover);
    color: var(--ov-text);
  }

  .tab[aria-selected="true"] {
    background: var(--ov-active);
    color: var(--ov-text);
    border-color: var(--ov-border);
  }

  .tab.pinned {
    position: sticky;
    left: 0;
    z-index: 1;
  }

  .tab.drop-before {
    box-shadow: inset 2px 0 0 var(--ov-accent);
  }

  .tab.drop-after {
    box-shadow: inset -2px 0 0 var(--ov-accent);
  }

  .pin {
    display: inline-flex;
    color: var(--ov-accent);
  }

  .tab-title {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .search-field {
    flex: 1;
    min-height: 24px;
    padding: 0 var(--space-2);
    background: rgb(0 0 0 / 25%);
    border-color: var(--ov-border);
    color: var(--ov-text);
    font-size: 0.86em;
  }

  .content {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .note-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: var(--space-1) var(--space-1) 0 var(--space-2);
  }

  .note-title {
    flex: 1;
    min-width: 0;
    min-height: 28px;
    padding: 0 var(--space-1);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--ov-text);
    font-size: 1.05em;
    font-weight: var(--weight-semibold);
  }

  .note-title:hover {
    border-color: var(--ov-border);
  }

  .note-title:focus {
    background: rgb(0 0 0 / 25%);
    border-color: var(--ov-accent);
  }

  .note-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .editor {
    display: block;
    width: 100%;
    height: 100%;
    min-height: 100%;
    padding: var(--space-2) var(--space-3);
    resize: none;
    background: rgb(0 0 0 / 22%);
    border: none;
    border-radius: 0;
    color: var(--ov-text);
    font-family: var(--font-mono);
    font-size: 0.9em;
    line-height: 1.5;
  }

  .editor:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 1px rgb(143 178 247 / 45%);
  }

  .format {
    flex: none;
    display: flex;
    align-items: center;
    gap: 1px;
    padding: 2px var(--space-1);
    border-top: 1px solid var(--ov-border);
    overflow-x: auto;
    scrollbar-width: none;
  }

  .format .small:not(.icon) {
    min-height: 24px;
    margin-left: var(--space-1);
    background: var(--ov-hover);
    border-color: var(--ov-border);
    color: var(--ov-text);
  }

  .save-state {
    margin: 0 0 0 auto;
    padding: 0 var(--space-1);
    color: var(--ov-subtle);
    font-size: 0.75em;
    white-space: nowrap;
  }

  .hits {
    flex: 1;
    margin: 0;
    padding: var(--space-1);
    overflow: auto;
    list-style: none;
  }

  .hits button {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    width: 100%;
    min-height: 0;
    padding: var(--space-1) var(--space-2);
    background: transparent;
    border-color: transparent;
    color: var(--ov-text);
    text-align: left;
    white-space: normal;
  }

  .hits button:hover,
  .hits button:focus-visible {
    background: var(--ov-hover);
  }

  .hit-title {
    font-weight: var(--weight-semibold);
  }

  .hit-snippet {
    color: var(--ov-muted);
    font-size: 0.85em;
    font-weight: var(--weight-regular);
  }

  .hits .none,
  .muted-line {
    padding: var(--space-2) var(--space-3);
    color: var(--ov-subtle);
  }

  .no-notes {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-3);
  }

  .no-notes p {
    margin: 0;
  }

  .hint-line {
    color: var(--ov-subtle);
    font-size: 0.85em;
  }

  .grip {
    position: absolute;
    right: 1px;
    bottom: 1px;
    display: inline-flex;
    padding: 2px;
    color: var(--ov-subtle);
    cursor: nwse-resize;
    opacity: 0.6;
  }

  .grip:hover {
    opacity: 1;
  }

  .overlay :global(:focus-visible) {
    outline-color: var(--ov-accent);
  }
</style>
