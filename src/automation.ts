// Typed access to the autoclicker and macros (ADR-050;
// src-tauri/src/automation/mod.rs). The page only asks: every click, key
// press and recording happens in Rust, while the `autoclicker` feature
// runs. The live status arrives on a channel (`watchAutomation`) whenever
// it changes. Every response is checked, so a Rust / TypeScript mismatch
// fails loudly; macros and profiles sent back are the parsed objects,
// changed by the page.
import { Channel, invoke } from "@tauri-apps/api/core";

export const AUTOMATION_STATUS = "automation_status";
export const WATCH_AUTOMATION = "watch_automation";
export const UNWATCH_AUTOMATION = "unwatch_automation";
export const AUTOMATION_CONFIG = "automation_config";
export const START_CLICKER = "start_clicker";
export const PLAY_MACRO = "play_macro";
export const START_RECORDING = "start_recording";
export const SET_AUTOMATION_PAUSED = "set_automation_paused";
export const STOP_AUTOMATION = "stop_automation";
export const SAVE_RECORDING = "save_recording";
export const DISCARD_RECORDING = "discard_recording";
export const LIST_MACROS = "list_macros";
export const GET_MACRO = "get_macro";
export const CREATE_MACRO = "create_macro";
export const SAVE_MACRO = "save_macro";
export const UPDATE_MACRO_INFO = "update_macro_info";
export const DUPLICATE_MACRO = "duplicate_macro";
export const DELETE_MACRO = "delete_macro";
export const CREATE_PROFILE = "create_profile";
export const SAVE_PROFILE = "save_profile";
export const DELETE_PROFILE = "delete_profile";
export const ACTIVATE_PROFILE = "activate_profile";
export const SET_DEFAULT_PROFILE = "set_default_profile";
export const SAVE_AUTOMATION_SETTINGS = "save_automation_settings";
export const EXPORT_AUTOMATION = "export_automation";
export const IMPORT_AUTOMATION = "import_automation";
export const CAPTURE_CURSOR_POSITION = "capture_cursor_position";
export const CAPTURE_ACTIVE_WINDOW = "capture_active_window";
export const SCREEN_LAYOUT = "screen_layout";

/** The built-in feature that runs automation. */
export const AUTOCLICKER_FEATURE = "autoclicker";

// ---------------------------------------------------------------- types

export const MOUSE_BUTTONS = ["left", "right", "middle", "back", "forward"] as const;
export type MouseButton = (typeof MOUSE_BUTTONS)[number];
export const PRESSES = ["down", "up"] as const;
export type Press = (typeof PRESSES)[number];

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Monitor {
  bounds: Rect;
  primary: boolean;
}

export interface WindowInfo {
  title: string;
  process: string;
  client: Rect;
}

export type ClickType = "single" | "double";
export type RateUnit = "cps" | "interval";
export type RandomMode = "off" | "range" | "variation";
export type TargetMode = "cursor" | "points" | "area";
export type PointOrder = "sequential" | "random";

export interface RandomInterval {
  mode: RandomMode;
  minMs: number;
  maxMs: number;
  variationPercent: number;
}

export interface ClickTarget {
  mode: TargetMode;
  points: Point[];
  order: PointOrder;
  area: Rect | null;
  spread: number;
}

export interface Batch {
  clicks: number;
  pauseMs: number;
}

export interface ClickerConfig {
  button: MouseButton;
  clickType: ClickType;
  intervalMs: number;
  rateUnit: RateUnit;
  random: RandomInterval;
  maxClicks: number | null;
  maxDurationMs: number | null;
  startDelayMs: number;
  batch: Batch | null;
  target: ClickTarget;
  holdMs: number;
  doubleGapMs: number;
  advanced: boolean;
}

export type ActionKind =
  | { type: "move"; x: number; y: number }
  | { type: "button"; button: MouseButton; state: Press; x: number; y: number }
  | { type: "click"; button: MouseButton; count: number; x: number | null; y: number | null }
  | { type: "scroll"; dx: number; dy: number; x: number | null; y: number | null }
  | { type: "key"; key: string; state: Press }
  | { type: "keys"; chord: string }
  | { type: "wait" }
  | { type: "pause" };

export type ActionType = ActionKind["type"];
export type Action = ActionKind & { delayMs: number };

export type Positioning = "absolute" | "cursor" | "window";

export interface Section {
  first: number;
  last: number;
}

export interface PlaybackSettings {
  repeat: number | null;
  stopAfterMs: number | null;
  speed: number;
  startDelayMs: number;
  repeatDelayMs: number;
  jitterPercent: number;
  positioning: Positioning;
  scaleToScreen: boolean;
  section: Section | null;
}

export type Mismatch = "warn" | "stop";

export interface TargetApp {
  process: string;
  title: string | null;
  onMismatch: Mismatch;
}

export interface RecordedOn {
  cursor: Point;
  window: WindowInfo | null;
  monitors: Monitor[];
}

export interface Binding {
  chord: string | null;
  enabled: boolean;
}

export interface Macro {
  formatVersion: number;
  appVersion: string;
  id: string;
  name: string;
  description: string;
  folder: string;
  favorite: boolean;
  shortcut: Binding;
  createdMs: number;
  modifiedMs: number;
  recordedOn: RecordedOn | null;
  target: TargetApp | null;
  playback: PlaybackSettings;
  actions: Action[];
}

export interface MacroSummary {
  id: string;
  name: string;
  description: string;
  folder: string;
  favorite: boolean;
  shortcut: Binding;
  actions: number;
  durationMs: number;
  createdMs: number;
  modifiedMs: number;
  target: string | null;
  hasPause: boolean;
}

export const HOTKEY_ACTIONS = [
  "toggle-clicker",
  "start-clicker",
  "pause-resume",
  "start-recording",
  "stop-recording",
  "play-macro",
  "stop-playback",
  "stop-all",
] as const;
export type HotkeyAction = (typeof HOTKEY_ACTIONS)[number];
/** The shortcuts a profile has (the emergency stop is in the settings). */
export const PROFILE_ACTIONS = HOTKEY_ACTIONS.filter((a) => a !== "stop-all");

export interface Profile {
  id: string;
  name: string;
  clicker: ClickerConfig;
  hotkeys: Partial<Record<HotkeyAction, Binding>>;
  macroId: string | null;
  playback: PlaybackSettings | null;
  createdMs: number;
  modifiedMs: number;
}

export type Coordinates = "absolute" | "relative" | "window";

export interface RecordingOptions {
  mouseMoves: boolean;
  mouseButtons: boolean;
  keyboard: boolean;
  scroll: boolean;
  coordinates: Coordinates;
  moveRateHz: number;
  minMovePx: number;
  ignoredKeys: string[];
  ignoredButtons: MouseButton[];
}

export interface Settings {
  defaultProfile: string;
  emergencyStop: Binding;
  failsafeCorners: boolean;
  dashboardCountdownMs: number;
  recording: RecordingOptions;
}

export const KEY_KINDS = ["modifier", "layout", "function", "media", "quiet", "other"] as const;
export type KeyKind = (typeof KEY_KINDS)[number];

export interface KeyView {
  code: string;
  label: string;
  kind: KeyKind;
}

export interface AutomationConfig {
  settings: Settings;
  profiles: Profile[];
  activeProfile: string;
  keys: KeyView[];
  suggestions: Partial<Record<HotkeyAction, string>>;
}

export type RunState = "countdown" | "running" | "paused";

export interface ClickerProgress {
  state: RunState;
  clicks: number;
  elapsedMs: number;
  countdownMs: number | null;
  cps: number;
  skipped: number;
  remaining: number | null;
}

export interface PlaybackProgress {
  state: RunState;
  repetition: number;
  repetitions: number | null;
  action: number;
  actions: number;
  label: string;
  progress: number | null;
  elapsedMs: number;
  remainingMs: number | null;
  countdownMs: number | null;
  skipped: number;
  warning: string | null;
  atPausePoint: boolean;
}

export type SessionView =
  | { kind: "clicker"; progress: ClickerProgress; targetCps: number; startedMs: number; profile: string }
  | { kind: "playback"; progress: PlaybackProgress; macroId: string; macroName: string; startedMs: number }
  | {
      kind: "recording";
      state: RunState;
      elapsedMs: number;
      countdownMs: number | null;
      actions: number;
      recent: Action[];
      skipped: number;
      startedMs: number;
    };

export interface LastSession {
  kind: "clicker" | "playback" | "recording";
  outcome: "completed" | "stopped" | "failsafe" | "failed";
  message: string;
  atMs: number;
}

export interface DraftView {
  actions: number;
  durationMs: number;
  skipped: number;
  recent: Action[];
  window: WindowInfo | null;
}

export type BindingOwner =
  | { kind: "settings" }
  | { kind: "profile"; action: HotkeyAction }
  | { kind: "macro"; id: string; name: string };

export interface HotkeyView {
  owner: BindingOwner;
  label: string;
  chord: string;
  /** The keys' names on this keyboard. */
  keys: string;
  registered: boolean;
  error: string | null;
}

export interface Support {
  available: boolean;
  backend: string;
  note: string | null;
}

export interface AutomationStatus {
  seq: number;
  /** The `autoclicker` feature runs. */
  running: boolean;
  support: Support;
  session: SessionView | null;
  last: LastSession | null;
  draft: DraftView | null;
  hotkeys: HotkeyView[];
  activeProfile: { id: string; name: string };
  activeWindow: WindowInfo | null;
  notice: string | null;
  storageError: string | null;
}

export interface SavedMacro {
  summary: MacroSummary;
  warnings: string[];
}

export interface MacroChanges {
  name?: string;
  description?: string;
  folder?: string;
  favorite?: boolean;
  shortcut?: Binding;
}

export interface ImportReport {
  macros: MacroSummary[];
  profiles: { id: string; name: string }[];
  problems: string[];
}

export interface ScreenLayout {
  monitors: Monitor[];
  bounds: Rect | null;
}

export interface CapturedPoint {
  point: Point;
  monitor: number | null;
  onSotyware: boolean;
}

export const ERROR_KINDS = [
  "invalid-input",
  "not-found",
  "busy",
  "feature-off",
  "unavailable",
  "storage",
  "confirm",
  "input",
  "file",
  "conflict",
] as const;
export type ErrorKind = (typeof ERROR_KINDS)[number];

/** A command's refusal, as Rust sent it. */
export class AutomationError extends Error {
  readonly kind: ErrorKind;

  constructor(kind: ErrorKind, message: string) {
    super(message);
    this.name = "AutomationError";
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

function optNum(what: string, v: Rec, k: string): number | null {
  return v[k] === null || v[k] === undefined ? null : num(what, v, k);
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

function arr(what: string, v: Rec, k: string): unknown[] {
  const x = v[k];
  if (!Array.isArray(x)) {
    throw invalid(what, v);
  }
  return x;
}

function parsePoint(value: unknown): Point {
  const v = rec("point", value);
  return { x: num("point", v, "x"), y: num("point", v, "y") };
}

function parseRect(value: unknown): Rect {
  const v = rec("rectangle", value);
  return {
    x: num("rectangle", v, "x"),
    y: num("rectangle", v, "y"),
    width: num("rectangle", v, "width"),
    height: num("rectangle", v, "height"),
  };
}

function parseMonitor(value: unknown): Monitor {
  const v = rec("monitor", value);
  return { bounds: parseRect(v["bounds"]), primary: bool("monitor", v, "primary") };
}

export function parseWindow(value: unknown): WindowInfo {
  const v = rec("window", value);
  return { title: str("window", v, "title"), process: str("window", v, "process"), client: parseRect(v["client"]) };
}

function optWindow(value: unknown): WindowInfo | null {
  return value === null || value === undefined ? null : parseWindow(value);
}

function parseBinding(value: unknown): Binding {
  const v = rec("shortcut", value);
  return { chord: optStr("shortcut", v, "chord"), enabled: bool("shortcut", v, "enabled") };
}

export function parseClicker(value: unknown): ClickerConfig {
  const w = "clicker settings";
  const v = rec(w, value);
  const r = rec(w, v["random"]);
  const t = rec(w, v["target"]);
  const b = v["batch"] === null ? null : rec(w, v["batch"]);
  return {
    button: oneOf(w, MOUSE_BUTTONS, v, "button"),
    clickType: oneOf(w, ["single", "double"] as const, v, "clickType"),
    intervalMs: num(w, v, "intervalMs"),
    rateUnit: oneOf(w, ["cps", "interval"] as const, v, "rateUnit"),
    random: {
      mode: oneOf(w, ["off", "range", "variation"] as const, r, "mode"),
      minMs: num(w, r, "minMs"),
      maxMs: num(w, r, "maxMs"),
      variationPercent: num(w, r, "variationPercent"),
    },
    maxClicks: optNum(w, v, "maxClicks"),
    maxDurationMs: optNum(w, v, "maxDurationMs"),
    startDelayMs: num(w, v, "startDelayMs"),
    batch: b === null ? null : { clicks: num(w, b, "clicks"), pauseMs: num(w, b, "pauseMs") },
    target: {
      mode: oneOf(w, ["cursor", "points", "area"] as const, t, "mode"),
      points: arr(w, t, "points").map(parsePoint),
      order: oneOf(w, ["sequential", "random"] as const, t, "order"),
      area: t["area"] === null ? null : parseRect(t["area"]),
      spread: num(w, t, "spread"),
    },
    holdMs: num(w, v, "holdMs"),
    doubleGapMs: num(w, v, "doubleGapMs"),
    advanced: bool(w, v, "advanced"),
  };
}

export function parseAction(value: unknown): Action {
  const w = "action";
  const v = rec(w, value);
  const delayMs = num(w, v, "delayMs");
  const optCoord = (k: "x" | "y"): number | null => optNum(w, v, k);
  switch (v["type"]) {
    case "move":
      return { delayMs, type: "move", x: num(w, v, "x"), y: num(w, v, "y") };
    case "button":
      return {
        delayMs,
        type: "button",
        button: oneOf(w, MOUSE_BUTTONS, v, "button"),
        state: oneOf(w, PRESSES, v, "state"),
        x: num(w, v, "x"),
        y: num(w, v, "y"),
      };
    case "click":
      return {
        delayMs,
        type: "click",
        button: oneOf(w, MOUSE_BUTTONS, v, "button"),
        count: num(w, v, "count"),
        x: optCoord("x"),
        y: optCoord("y"),
      };
    case "scroll":
      return { delayMs, type: "scroll", dx: num(w, v, "dx"), dy: num(w, v, "dy"), x: optCoord("x"), y: optCoord("y") };
    case "key":
      return { delayMs, type: "key", key: str(w, v, "key"), state: oneOf(w, PRESSES, v, "state") };
    case "keys":
      return { delayMs, type: "keys", chord: str(w, v, "chord") };
    case "wait":
      return { delayMs, type: "wait" };
    case "pause":
      return { delayMs, type: "pause" };
    default:
      throw invalid(w, value);
  }
}

export function parsePlayback(value: unknown): PlaybackSettings {
  const w = "playback settings";
  const v = rec(w, value);
  const s = v["section"] === null ? null : rec(w, v["section"]);
  return {
    repeat: optNum(w, v, "repeat"),
    stopAfterMs: optNum(w, v, "stopAfterMs"),
    speed: num(w, v, "speed"),
    startDelayMs: num(w, v, "startDelayMs"),
    repeatDelayMs: num(w, v, "repeatDelayMs"),
    jitterPercent: num(w, v, "jitterPercent"),
    positioning: oneOf(w, ["absolute", "cursor", "window"] as const, v, "positioning"),
    scaleToScreen: bool(w, v, "scaleToScreen"),
    section: s === null ? null : { first: num(w, s, "first"), last: num(w, s, "last") },
  };
}

export function parseMacro(value: unknown): Macro {
  const w = "macro";
  const v = rec(w, value);
  const t = v["target"] === null ? null : rec(w, v["target"]);
  const r = v["recordedOn"] === null ? null : rec(w, v["recordedOn"]);
  return {
    formatVersion: num(w, v, "formatVersion"),
    appVersion: str(w, v, "appVersion"),
    id: str(w, v, "id"),
    name: str(w, v, "name"),
    description: str(w, v, "description"),
    folder: str(w, v, "folder"),
    favorite: bool(w, v, "favorite"),
    shortcut: parseBinding(v["shortcut"]),
    createdMs: num(w, v, "createdMs"),
    modifiedMs: num(w, v, "modifiedMs"),
    recordedOn:
      r === null
        ? null
        : { cursor: parsePoint(r["cursor"]), window: optWindow(r["window"]), monitors: arr(w, r, "monitors").map(parseMonitor) },
    target:
      t === null
        ? null
        : { process: str(w, t, "process"), title: optStr(w, t, "title"), onMismatch: oneOf(w, ["warn", "stop"] as const, t, "onMismatch") },
    playback: parsePlayback(v["playback"]),
    actions: arr(w, v, "actions").map(parseAction),
  };
}

export function parseSummary(value: unknown): MacroSummary {
  const w = "macro summary";
  const v = rec(w, value);
  return {
    id: str(w, v, "id"),
    name: str(w, v, "name"),
    description: str(w, v, "description"),
    folder: str(w, v, "folder"),
    favorite: bool(w, v, "favorite"),
    shortcut: parseBinding(v["shortcut"]),
    actions: num(w, v, "actions"),
    durationMs: num(w, v, "durationMs"),
    createdMs: num(w, v, "createdMs"),
    modifiedMs: num(w, v, "modifiedMs"),
    target: optStr(w, v, "target"),
    hasPause: bool(w, v, "hasPause"),
  };
}

export function parseProfile(value: unknown): Profile {
  const w = "profile";
  const v = rec(w, value);
  const h = rec(w, v["hotkeys"]);
  const hotkeys: Partial<Record<HotkeyAction, Binding>> = {};
  for (const [k, b] of Object.entries(h)) {
    if (!(HOTKEY_ACTIONS as readonly string[]).includes(k)) {
      throw invalid(w, value);
    }
    hotkeys[k as HotkeyAction] = parseBinding(b);
  }
  return {
    id: str(w, v, "id"),
    name: str(w, v, "name"),
    clicker: parseClicker(v["clicker"]),
    hotkeys,
    macroId: optStr(w, v, "macroId"),
    playback: v["playback"] === null ? null : parsePlayback(v["playback"]),
    createdMs: num(w, v, "createdMs"),
    modifiedMs: num(w, v, "modifiedMs"),
  };
}

export function parseSettings(value: unknown): Settings {
  const w = "settings";
  const v = rec(w, value);
  const r = rec(w, v["recording"]);
  return {
    defaultProfile: str(w, v, "defaultProfile"),
    emergencyStop: parseBinding(v["emergencyStop"]),
    failsafeCorners: bool(w, v, "failsafeCorners"),
    dashboardCountdownMs: num(w, v, "dashboardCountdownMs"),
    recording: {
      mouseMoves: bool(w, r, "mouseMoves"),
      mouseButtons: bool(w, r, "mouseButtons"),
      keyboard: bool(w, r, "keyboard"),
      scroll: bool(w, r, "scroll"),
      coordinates: oneOf(w, ["absolute", "relative", "window"] as const, r, "coordinates"),
      moveRateHz: num(w, r, "moveRateHz"),
      minMovePx: num(w, r, "minMovePx"),
      ignoredKeys: arr(w, r, "ignoredKeys").map((k) => {
        if (typeof k !== "string") {
          throw invalid(w, value);
        }
        return k;
      }),
      ignoredButtons: arr(w, r, "ignoredButtons").map((b) => {
        if (!(MOUSE_BUTTONS as readonly unknown[]).includes(b)) {
          throw invalid(w, value);
        }
        return b as MouseButton;
      }),
    },
  };
}

export function parseConfig(value: unknown): AutomationConfig {
  const w = "automation configuration";
  const v = rec(w, value);
  const s = rec(w, v["suggestions"]);
  const suggestions: Partial<Record<HotkeyAction, string>> = {};
  for (const [k, c] of Object.entries(s)) {
    if (!(HOTKEY_ACTIONS as readonly string[]).includes(k) || typeof c !== "string") {
      throw invalid(w, s);
    }
    suggestions[k as HotkeyAction] = c;
  }
  return {
    settings: parseSettings(v["settings"]),
    profiles: arr(w, v, "profiles").map(parseProfile),
    activeProfile: str(w, v, "activeProfile"),
    keys: arr(w, v, "keys").map((k) => {
      const kv = rec(w, k);
      return { code: str(w, kv, "code"), label: str(w, kv, "label"), kind: oneOf(w, KEY_KINDS, kv, "kind") };
    }),
    suggestions,
  };
}

const RUN_STATES = ["countdown", "running", "paused"] as const;

function parseSession(value: unknown): SessionView {
  const w = "session";
  const v = rec(w, value);
  switch (v["kind"]) {
    case "clicker": {
      const p = rec(w, v["progress"]);
      return {
        kind: "clicker",
        progress: {
          state: oneOf(w, RUN_STATES, p, "state"),
          clicks: num(w, p, "clicks"),
          elapsedMs: num(w, p, "elapsedMs"),
          countdownMs: optNum(w, p, "countdownMs"),
          cps: num(w, p, "cps"),
          skipped: num(w, p, "skipped"),
          remaining: optNum(w, p, "remaining"),
        },
        targetCps: num(w, v, "targetCps"),
        startedMs: num(w, v, "startedMs"),
        profile: str(w, v, "profile"),
      };
    }
    case "playback": {
      const p = rec(w, v["progress"]);
      return {
        kind: "playback",
        progress: {
          state: oneOf(w, RUN_STATES, p, "state"),
          repetition: num(w, p, "repetition"),
          repetitions: optNum(w, p, "repetitions"),
          action: num(w, p, "action"),
          actions: num(w, p, "actions"),
          label: str(w, p, "label"),
          progress: optNum(w, p, "progress"),
          elapsedMs: num(w, p, "elapsedMs"),
          remainingMs: optNum(w, p, "remainingMs"),
          countdownMs: optNum(w, p, "countdownMs"),
          skipped: num(w, p, "skipped"),
          warning: optStr(w, p, "warning"),
          atPausePoint: bool(w, p, "atPausePoint"),
        },
        macroId: str(w, v, "macroId"),
        macroName: str(w, v, "macroName"),
        startedMs: num(w, v, "startedMs"),
      };
    }
    case "recording":
      return {
        kind: "recording",
        state: oneOf(w, RUN_STATES, v, "state"),
        elapsedMs: num(w, v, "elapsedMs"),
        countdownMs: optNum(w, v, "countdownMs"),
        actions: num(w, v, "actions"),
        recent: arr(w, v, "recent").map(parseAction),
        skipped: num(w, v, "skipped"),
        startedMs: num(w, v, "startedMs"),
      };
    default:
      throw invalid(w, value);
  }
}

function parseOwner(value: unknown): BindingOwner {
  const v = rec("shortcut owner", value);
  switch (v["kind"]) {
    case "settings":
      return { kind: "settings" };
    case "profile":
      return { kind: "profile", action: oneOf("shortcut owner", HOTKEY_ACTIONS, v, "action") };
    case "macro":
      return { kind: "macro", id: str("shortcut owner", v, "id"), name: str("shortcut owner", v, "name") };
    default:
      throw invalid("shortcut owner", value);
  }
}

export function parseStatus(value: unknown): AutomationStatus {
  const w = "automation status";
  const v = rec(w, value);
  const s = rec(w, v["support"]);
  const a = rec(w, v["activeProfile"]);
  const last = v["last"] === null ? null : rec(w, v["last"]);
  const d = v["draft"] === null ? null : rec(w, v["draft"]);
  return {
    seq: num(w, v, "seq"),
    running: bool(w, v, "running"),
    support: { available: bool(w, s, "available"), backend: str(w, s, "backend"), note: optStr(w, s, "note") },
    session: v["session"] === null ? null : parseSession(v["session"]),
    last:
      last === null
        ? null
        : {
            kind: oneOf(w, ["clicker", "playback", "recording"] as const, last, "kind"),
            outcome: oneOf(w, ["completed", "stopped", "failsafe", "failed"] as const, last, "outcome"),
            message: str(w, last, "message"),
            atMs: num(w, last, "atMs"),
          },
    draft:
      d === null
        ? null
        : {
            actions: num(w, d, "actions"),
            durationMs: num(w, d, "durationMs"),
            skipped: num(w, d, "skipped"),
            recent: arr(w, d, "recent").map(parseAction),
            window: optWindow(d["window"]),
          },
    hotkeys: arr(w, v, "hotkeys").map((h) => {
      const hv = rec(w, h);
      return {
        owner: parseOwner(hv["owner"]),
        label: str(w, hv, "label"),
        chord: str(w, hv, "chord"),
        keys: str(w, hv, "keys"),
        registered: bool(w, hv, "registered"),
        error: optStr(w, hv, "error"),
      };
    }),
    activeProfile: { id: str(w, a, "id"), name: str(w, a, "name") },
    activeWindow: optWindow(v["activeWindow"]),
    notice: optStr(w, v, "notice"),
    storageError: optStr(w, v, "storageError"),
  };
}

function parseImport(value: unknown): ImportReport {
  const w = "import report";
  const v = rec(w, value);
  return {
    macros: arr(w, v, "macros").map(parseSummary),
    profiles: arr(w, v, "profiles").map((p) => {
      const pv = rec(w, p);
      return { id: str(w, pv, "id"), name: str(w, pv, "name") };
    }),
    problems: arr(w, v, "problems").map((p) => {
      if (typeof p !== "string") {
        throw invalid(w, value);
      }
      return p;
    }),
  };
}

// ------------------------------------------------------------- commands

function toError(e: unknown): Error {
  if (typeof e === "object" && e !== null && !Array.isArray(e)) {
    const v = e as Rec;
    if ((ERROR_KINDS as readonly unknown[]).includes(v["kind"]) && typeof v["message"] === "string") {
      return new AutomationError(v["kind"] as ErrorKind, v["message"]);
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

export async function automationStatus(): Promise<AutomationStatus> {
  return parseStatus(await call(AUTOMATION_STATUS));
}

/** Sends the status to `onStatus` now and whenever it changes; the
 * subscription's id, for `unwatchAutomation`. */
export async function watchAutomation(onStatus: (s: AutomationStatus) => void): Promise<number> {
  const channel = new Channel<unknown>();
  channel.onmessage = (m) => {
    try {
      onStatus(parseStatus(m));
    } catch (e) {
      console.error(e);
    }
  };
  const id = await call(WATCH_AUTOMATION, { onEvent: channel });
  if (typeof id !== "number") {
    throw invalid("subscription", id);
  }
  return id;
}

export async function unwatchAutomation(subscription: number): Promise<void> {
  await call(UNWATCH_AUTOMATION, { subscription });
}

export async function automationConfig(): Promise<AutomationConfig> {
  return parseConfig(await call(AUTOMATION_CONFIG));
}

/** Starts the active profile's autoclicker. `confirmed`: the user agreed
 * to an endless run without an emergency stop (error kind `confirm`). */
export async function startClicker(confirmed = false): Promise<void> {
  await call(START_CLICKER, { fromDashboard: true, confirmed });
}

/** Plays a macro with its own settings, or (`null`) the active profile's
 * macro with the profile's. */
export async function playMacro(id: string | null, confirmed = false): Promise<void> {
  await call(PLAY_MACRO, { id, fromDashboard: true, confirmed });
}

export async function startRecording(): Promise<void> {
  await call(START_RECORDING, { fromDashboard: true });
}

export async function setAutomationPaused(paused: boolean): Promise<void> {
  await call(SET_AUTOMATION_PAUSED, { paused });
}

/** Stops whatever runs; resolves once every key and button is released. */
export async function stopAutomation(): Promise<void> {
  await call(STOP_AUTOMATION);
}

export async function saveRecording(name: string, description: string, folder: string): Promise<MacroSummary> {
  return parseSummary(await call(SAVE_RECORDING, { name, description, folder }));
}

export async function discardRecording(): Promise<void> {
  await call(DISCARD_RECORDING);
}

export async function listMacros(): Promise<MacroSummary[]> {
  const v = await call(LIST_MACROS);
  if (!Array.isArray(v)) {
    throw invalid("macro list", v);
  }
  return v.map(parseSummary);
}

export async function getMacro(id: string): Promise<Macro> {
  return parseMacro(await call(GET_MACRO, { id }));
}

export async function createMacro(name: string, description = "", folder = ""): Promise<Macro> {
  return parseMacro(await call(CREATE_MACRO, { name, description, folder }));
}

export async function saveMacro(m: Macro): Promise<SavedMacro> {
  const v = rec("saved macro", await call(SAVE_MACRO, { document: m }));
  const warnings = arr("saved macro", v, "warnings");
  return {
    summary: parseSummary(v["summary"]),
    warnings: warnings.map((x) => {
      if (typeof x !== "string") {
        throw invalid("saved macro", v);
      }
      return x;
    }),
  };
}

export async function updateMacroInfo(id: string, changes: MacroChanges): Promise<MacroSummary> {
  return parseSummary(await call(UPDATE_MACRO_INFO, { id, changes }));
}

export async function duplicateMacro(id: string): Promise<MacroSummary> {
  return parseSummary(await call(DUPLICATE_MACRO, { id }));
}

export async function deleteMacro(id: string): Promise<void> {
  await call(DELETE_MACRO, { id });
}

export async function createProfile(name: string, from: string | null = null): Promise<Profile> {
  return parseProfile(await call(CREATE_PROFILE, { name, from }));
}

export async function saveProfile(profile: Profile): Promise<Profile> {
  return parseProfile(await call(SAVE_PROFILE, { profile }));
}

export async function deleteProfile(id: string): Promise<void> {
  await call(DELETE_PROFILE, { id });
}

export async function activateProfile(id: string): Promise<void> {
  await call(ACTIVATE_PROFILE, { id });
}

export async function setDefaultProfile(id: string): Promise<void> {
  await call(SET_DEFAULT_PROFILE, { id });
}

export async function saveAutomationSettings(settings: Settings): Promise<Settings> {
  return parseSettings(await call(SAVE_AUTOMATION_SETTINGS, { settings }));
}

/** Rust opens the save dialog; the file's name, or `null` if cancelled. */
export async function exportAutomation(macros: string[], profiles: string[]): Promise<string | null> {
  const v = await call(EXPORT_AUTOMATION, { macros, profiles });
  if (v !== null && typeof v !== "string") {
    throw invalid("export result", v);
  }
  return v;
}

/** Rust opens the open dialog; what was imported, or `null` if cancelled. */
export async function importAutomation(): Promise<ImportReport | null> {
  const v = await call(IMPORT_AUTOMATION);
  return v === null ? null : parseImport(v);
}

export async function captureCursorPosition(delayMs: number): Promise<CapturedPoint> {
  const v = rec("captured point", await call(CAPTURE_CURSOR_POSITION, { delayMs }));
  return {
    point: parsePoint(v["point"]),
    monitor: optNum("captured point", v, "monitor"),
    onSotyware: bool("captured point", v, "onSotyware"),
  };
}

export async function captureActiveWindow(delayMs: number): Promise<WindowInfo> {
  return parseWindow(await call(CAPTURE_ACTIVE_WINDOW, { delayMs }));
}

export async function screenLayout(): Promise<ScreenLayout> {
  const v = rec("screen layout", await call(SCREEN_LAYOUT));
  return {
    monitors: arr("screen layout", v, "monitors").map(parseMonitor),
    bounds: v["bounds"] === null ? null : parseRect(v["bounds"]),
  };
}
