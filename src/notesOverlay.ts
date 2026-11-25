// Typed access to the gaming notes overlay (ADR-052;
// src-tauri/src/overlay/mod.rs). Notes, settings and the window live in
// Rust: the overlay's page edits notes through these commands, the
// dashboard's Settings › Notes overlay configures it, and both watch the
// status on a channel (`watchOverlay`). Every response is checked, so a
// Rust / TypeScript mismatch fails loudly.
import { Channel, invoke } from "@tauri-apps/api/core";
import { KEY_KINDS, type Binding, type KeyView } from "./automation";

export const WATCH_OVERLAY = "watch_overlay";
export const UNWATCH_OVERLAY = "unwatch_overlay";
export const OVERLAY_CONFIG = "overlay_config";
export const SAVE_OVERLAY_SETTINGS = "save_overlay_settings";
export const RESET_OVERLAY = "reset_overlay";
export const SET_OVERLAY_VISIBLE = "set_overlay_visible";
export const SET_OVERLAY_MODE = "set_overlay_mode";
export const SET_OVERLAY_COLLAPSED = "set_overlay_collapsed";
export const SET_OVERLAY_OPACITY = "set_overlay_opacity";
export const OPEN_OVERLAY_SETTINGS = "open_overlay_settings";
export const OPEN_NOTE_LINK = "open_note_link";
export const LIST_NOTES = "list_notes";
export const CREATE_NOTE = "create_note";
export const SAVE_NOTE = "save_note";
export const DELETE_NOTE = "delete_note";
export const DUPLICATE_NOTE = "duplicate_note";
export const SET_NOTE_PINNED = "set_note_pinned";
export const MOVE_NOTE = "move_note";
export const SELECT_NOTE = "select_note";

/** The built-in feature the overlay runs as. */
export const OVERLAY_FEATURE = "notes-overlay";

/** A note's title and text, in characters: Rust's limits
 * (`MAX_TITLE_CHARS`, `MAX_BODY_CHARS` in overlay/model.rs). A field's
 * `maxlength` counts UTF-16 units, so it is at least as strict. */
export const TITLE_MAX = 120;
export const BODY_MAX = 50_000;

// ---------------------------------------------------------------- types

export const MODES = ["interactive", "passive"] as const;
export type Mode = (typeof MODES)[number];
export const TEXT_SIZES = ["small", "medium", "large"] as const;
export type TextSize = (typeof TEXT_SIZES)[number];
export const ANCHORS = [
  "top-left",
  "top-right",
  "bottom-left",
  "bottom-right",
  "center",
  "center-left",
  "center-right",
  "mid-center-left",
  "mid-center-right",
] as const;
export type Anchor = (typeof ANCHORS)[number];
export const NEW_NOTE_TEMPLATES = ["blank", "checklist"] as const;
export type NewNoteTemplate = (typeof NEW_NOTE_TEMPLATES)[number];
export const OPEN_NOTES = ["last", "first-pinned"] as const;
export type OpenNote = (typeof OPEN_NOTES)[number];
export const OVERLAY_ACTIONS = ["toggle-visible", "toggle-mode", "new-note", "previous-note", "next-note"] as const;
export type OverlayAction = (typeof OVERLAY_ACTIONS)[number];
export const FOCUS_TARGETS = ["title", "content"] as const;
export type FocusTarget = (typeof FOCUS_TARGETS)[number];

export const ACTION_LABELS: Record<OverlayAction, string> = {
  "toggle-visible": "Show or hide the overlay",
  "toggle-mode": "Switch between interactive and passive",
  "new-note": "New note",
  "previous-note": "Previous note",
  "next-note": "Next note",
};

export interface OverlaySettings {
  formatVersion: number;
  /** The background's opacity in percent; the text stays opaque. */
  opacity: number;
  textSize: TextSize;
  /** The size a new overlay opens with, logical pixels. */
  width: number;
  height: number;
  anchor: Anchor;
  rememberPlacement: boolean;
  defaultMode: Mode;
  showAtStartup: boolean;
  newNote: NewNoteTemplate;
  openNote: OpenNote;
  shortcuts: Partial<Record<OverlayAction, Binding>>;
}

export interface OverlayHotkeyView {
  action: OverlayAction;
  label: string;
  chord: string;
  /** The keys' names on this keyboard. */
  keys: string;
  registered: boolean;
  error: string | null;
}

export interface OverlayStatus {
  seq: number;
  /** The `notes-overlay` feature runs. */
  running: boolean;
  visible: boolean;
  mode: Mode;
  collapsed: boolean;
  opacity: number;
  textSize: TextSize;
  activeNote: string | null;
  /** Bumped when Rust chose another note (a shortcut). */
  activeSeq: number;
  /** Bumped when Rust changed the notes (a note created by shortcut). */
  notesSeq: number;
  /** Bumped when the page should give the keyboard to `focusTarget`. */
  focusSeq: number;
  focusTarget: FocusTarget;
  /** Bumped when the overlay asked the dashboard to open its settings. */
  settingsSeq: number;
  noteCount: number;
  hotkeys: OverlayHotkeyView[];
  shortcutsNote: string | null;
  notice: string | null;
  storageError: string | null;
}

export interface OverlayLimits {
  minWidth: number;
  minHeight: number;
  maxWidth: number;
  maxHeight: number;
  minOpacity: number;
  maxOpacity: number;
  maxNotes: number;
  maxTitleChars: number;
  maxBodyChars: number;
}

export interface OverlayConfig {
  settings: OverlaySettings;
  suggestions: Partial<Record<OverlayAction, string>>;
  keys: KeyView[];
  limits: OverlayLimits;
}

export interface Note {
  id: string;
  title: string;
  /** Markdown. */
  body: string;
  pinned: boolean;
  position: number;
  createdMs: number;
  updatedMs: number;
  revision: number;
}

export interface NoteOrder {
  id: string;
  pinned: boolean;
  position: number;
}

export interface NotesList {
  /** In display order. */
  notes: Note[];
  activeNote: string | null;
}

export interface SavedNote {
  revision: number;
  updatedMs: number;
}

export const ERROR_KINDS = [
  "invalid-input",
  "not-found",
  "conflict",
  "limit",
  "feature-off",
  "storage",
  "window",
] as const;
export type ErrorKind = (typeof ERROR_KINDS)[number];

/** A command's refusal, as Rust sent it. */
export class OverlayError extends Error {
  readonly kind: ErrorKind;

  constructor(kind: ErrorKind, message: string) {
    super(message);
    this.name = "OverlayError";
    this.kind = kind;
  }
}

// -------------------------------------------------------------- parsing

type Rec = Record<string, unknown>;

function invalid(what: string, value: unknown): Error {
  return new Error(`invalid ${what}: ${JSON.stringify(value)?.slice(0, 300)}`);
}

function rec(what: string, value: unknown): Rec {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalid(what, value);
  }
  return value as Rec;
}

function num(what: string, v: Rec, k: string): number {
  const x = v[k];
  if (typeof x !== "number" || !Number.isFinite(x)) {
    throw invalid(what, v);
  }
  return x;
}

function str(what: string, v: Rec, k: string): string {
  const x = v[k];
  if (typeof x !== "string") {
    throw invalid(what, v);
  }
  return x;
}

function optStr(what: string, v: Rec, k: string): string | null {
  return v[k] === null || v[k] === undefined ? null : str(what, v, k);
}

function bool(what: string, v: Rec, k: string): boolean {
  const x = v[k];
  if (typeof x !== "boolean") {
    throw invalid(what, v);
  }
  return x;
}

function oneOf<T extends string>(what: string, list: readonly T[], v: Rec, k: string): T {
  const x = v[k];
  if (!(list as readonly unknown[]).includes(x)) {
    throw invalid(what, v);
  }
  return x as T;
}

function arr(what: string, v: unknown): unknown[] {
  if (!Array.isArray(v)) {
    throw invalid(what, v);
  }
  return v;
}

function parseBinding(value: unknown): Binding {
  const v = rec("shortcut", value);
  const chord = v["chord"];
  if (chord !== null && typeof chord !== "string") {
    throw invalid("shortcut", value);
  }
  return { chord, enabled: bool("shortcut", v, "enabled") };
}

export function parseSettings(value: unknown): OverlaySettings {
  const w = "overlay settings";
  const v = rec(w, value);
  const s = rec(w, v["shortcuts"]);
  const shortcuts: Partial<Record<OverlayAction, Binding>> = {};
  for (const [k, b] of Object.entries(s)) {
    if (!(OVERLAY_ACTIONS as readonly string[]).includes(k)) {
      throw invalid(w, value);
    }
    shortcuts[k as OverlayAction] = parseBinding(b);
  }
  return {
    formatVersion: num(w, v, "formatVersion"),
    opacity: num(w, v, "opacity"),
    textSize: oneOf(w, TEXT_SIZES, v, "textSize"),
    width: num(w, v, "width"),
    height: num(w, v, "height"),
    anchor: oneOf(w, ANCHORS, v, "anchor"),
    rememberPlacement: bool(w, v, "rememberPlacement"),
    defaultMode: oneOf(w, MODES, v, "defaultMode"),
    showAtStartup: bool(w, v, "showAtStartup"),
    newNote: oneOf(w, NEW_NOTE_TEMPLATES, v, "newNote"),
    openNote: oneOf(w, OPEN_NOTES, v, "openNote"),
    shortcuts,
  };
}

function parseHotkey(value: unknown): OverlayHotkeyView {
  const w = "overlay shortcut";
  const v = rec(w, value);
  return {
    action: oneOf(w, OVERLAY_ACTIONS, v, "action"),
    label: str(w, v, "label"),
    chord: str(w, v, "chord"),
    keys: str(w, v, "keys"),
    registered: bool(w, v, "registered"),
    error: optStr(w, v, "error"),
  };
}

export function parseStatus(value: unknown): OverlayStatus {
  const w = "overlay status";
  const v = rec(w, value);
  return {
    seq: num(w, v, "seq"),
    running: bool(w, v, "running"),
    visible: bool(w, v, "visible"),
    mode: oneOf(w, MODES, v, "mode"),
    collapsed: bool(w, v, "collapsed"),
    opacity: num(w, v, "opacity"),
    textSize: oneOf(w, TEXT_SIZES, v, "textSize"),
    activeNote: optStr(w, v, "activeNote"),
    activeSeq: num(w, v, "activeSeq"),
    notesSeq: num(w, v, "notesSeq"),
    focusSeq: num(w, v, "focusSeq"),
    focusTarget: oneOf(w, FOCUS_TARGETS, v, "focusTarget"),
    settingsSeq: num(w, v, "settingsSeq"),
    noteCount: num(w, v, "noteCount"),
    hotkeys: arr(w, v["hotkeys"]).map(parseHotkey),
    shortcutsNote: optStr(w, v, "shortcutsNote"),
    notice: optStr(w, v, "notice"),
    storageError: optStr(w, v, "storageError"),
  };
}

export function parseConfig(value: unknown): OverlayConfig {
  const w = "overlay configuration";
  const v = rec(w, value);
  const sugg = rec(w, v["suggestions"]);
  const suggestions: Partial<Record<OverlayAction, string>> = {};
  for (const [k, c] of Object.entries(sugg)) {
    if (!(OVERLAY_ACTIONS as readonly string[]).includes(k) || typeof c !== "string") {
      throw invalid(w, value);
    }
    suggestions[k as OverlayAction] = c;
  }
  const l = rec(w, v["limits"]);
  return {
    settings: parseSettings(v["settings"]),
    suggestions,
    keys: arr(w, v["keys"]).map((k) => {
      const kv = rec(w, k);
      return { code: str(w, kv, "code"), label: str(w, kv, "label"), kind: oneOf(w, KEY_KINDS, kv, "kind") };
    }),
    limits: {
      minWidth: num(w, l, "minWidth"),
      minHeight: num(w, l, "minHeight"),
      maxWidth: num(w, l, "maxWidth"),
      maxHeight: num(w, l, "maxHeight"),
      minOpacity: num(w, l, "minOpacity"),
      maxOpacity: num(w, l, "maxOpacity"),
      maxNotes: num(w, l, "maxNotes"),
      maxTitleChars: num(w, l, "maxTitleChars"),
      maxBodyChars: num(w, l, "maxBodyChars"),
    },
  };
}

export function parseNote(value: unknown): Note {
  const w = "note";
  const v = rec(w, value);
  return {
    id: str(w, v, "id"),
    title: str(w, v, "title"),
    body: str(w, v, "body"),
    pinned: bool(w, v, "pinned"),
    position: num(w, v, "position"),
    createdMs: num(w, v, "createdMs"),
    updatedMs: num(w, v, "updatedMs"),
    revision: num(w, v, "revision"),
  };
}

function parseOrder(value: unknown): NoteOrder[] {
  return arr("note order", value).map((o) => {
    const v = rec("note order", o);
    return { id: str("note order", v, "id"), pinned: bool("note order", v, "pinned"), position: num("note order", v, "position") };
  });
}

// ------------------------------------------------------------- commands

function toError(e: unknown): Error {
  if (typeof e === "object" && e !== null && !Array.isArray(e)) {
    const v = e as Rec;
    if ((ERROR_KINDS as readonly unknown[]).includes(v["kind"]) && typeof v["message"] === "string") {
      return new OverlayError(v["kind"] as ErrorKind, v["message"]);
    }
  }
  return e instanceof Error ? e : new Error(String(e));
}

async function call(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  try {
    return await invoke<unknown>(cmd, args);
  } catch (e) {
    throw toError(e);
  }
}

/** Sends the status to `onStatus` now and whenever it changes; the
 * subscription's id, for `unwatchOverlay`. */
export async function watchOverlay(onStatus: (s: OverlayStatus) => void): Promise<number> {
  const channel = new Channel<unknown>();
  channel.onmessage = (m) => {
    try {
      onStatus(parseStatus(m));
    } catch (e) {
      console.error(e);
    }
  };
  const id = await call(WATCH_OVERLAY, { onEvent: channel });
  if (typeof id !== "number") {
    throw invalid("subscription", id);
  }
  return id;
}

export async function unwatchOverlay(subscription: number): Promise<void> {
  await call(UNWATCH_OVERLAY, { subscription });
}

export async function overlayConfig(): Promise<OverlayConfig> {
  return parseConfig(await call(OVERLAY_CONFIG));
}

export async function saveOverlaySettings(settings: OverlaySettings): Promise<OverlaySettings> {
  return parseSettings(await call(SAVE_OVERLAY_SETTINGS, { settings }));
}

/** Back to the default settings; the notes stay. */
export async function resetOverlay(): Promise<OverlaySettings> {
  return parseSettings(await call(RESET_OVERLAY));
}

export async function setOverlayVisible(visible: boolean): Promise<OverlayStatus> {
  return parseStatus(await call(SET_OVERLAY_VISIBLE, { visible }));
}

export async function setOverlayMode(mode: Mode): Promise<OverlayStatus> {
  return parseStatus(await call(SET_OVERLAY_MODE, { mode }));
}

export async function setOverlayCollapsed(collapsed: boolean): Promise<OverlayStatus> {
  return parseStatus(await call(SET_OVERLAY_COLLAPSED, { collapsed }));
}

export async function setOverlayOpacity(opacity: number): Promise<OverlayStatus> {
  return parseStatus(await call(SET_OVERLAY_OPACITY, { opacity }));
}

/** Shows the dashboard at Settings › Notes overlay. */
export async function openOverlaySettings(): Promise<void> {
  await call(OPEN_OVERLAY_SETTINGS);
}

/** Opens a web address written in note `id` in the browser. */
export async function openNoteLink(id: string, url: string): Promise<void> {
  await call(OPEN_NOTE_LINK, { id, url });
}

export async function listNotes(): Promise<NotesList> {
  const v = rec("notes", await call(LIST_NOTES));
  return { notes: arr("notes", v["notes"]).map(parseNote), activeNote: optStr("notes", v, "activeNote") };
}

export async function createNote(title: string | null = null): Promise<Note> {
  return parseNote(await call(CREATE_NOTE, { title }));
}

export async function saveNote(id: string, title: string, body: string, revision: number): Promise<SavedNote> {
  const v = rec("saved note", await call(SAVE_NOTE, { id, title, body, revision }));
  return { revision: num("saved note", v, "revision"), updatedMs: num("saved note", v, "updatedMs") };
}

/** Deletes a note; the note to show now. */
export async function deleteNote(id: string): Promise<string | null> {
  return optStr("deleted note", rec("deleted note", await call(DELETE_NOTE, { id })), "activeNote");
}

export async function duplicateNote(id: string): Promise<{ note: Note; order: NoteOrder[] }> {
  const v = rec("copy", await call(DUPLICATE_NOTE, { id }));
  return { note: parseNote(v["note"]), order: parseOrder(v["order"]) };
}

export async function setNotePinned(id: string, pinned: boolean): Promise<NoteOrder[]> {
  return parseOrder(await call(SET_NOTE_PINNED, { id, pinned }));
}

export async function moveNote(id: string, index: number): Promise<NoteOrder[]> {
  return parseOrder(await call(MOVE_NOTE, { id, index }));
}

export async function selectNote(id: string): Promise<void> {
  await call(SELECT_NOTE, { id });
}
