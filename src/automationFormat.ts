// Pure helpers of the Autoclicker page (ADR-050): durations, rates,
// shortcuts from key presses, action descriptions and the macro timeline.
// No IPC and no DOM, so every rule here is unit-tested
// (src/automationFormat.test.ts). The final word on every value is Rust's;
// these only give immediate feedback.
import type {
  Action,
  ActionKind,
  ActionType,
  ClickerConfig,
  HotkeyAction,
  KeyKind,
  KeyView,
  MouseButton,
  PlaybackSettings,
} from "./automation";
import { formatCount } from "./format";

const MAC = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
const WINDOWS = typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);

/** The Meta modifier's name on this system. */
export const META_NAME = MAC ? "⌘" : WINDOWS ? "Win" : "Super";

/** The number in a number field, or `null` when it is empty or not one
 * (read from the text: `valueAsNumber` differs between engines). */
export function numberOf(input: HTMLInputElement): number | null {
  const raw = input.value.trim();
  if (raw === "") {
    return null;
  }
  const v = Number(raw);
  return Number.isFinite(v) ? v : null;
}

/** `1:02:03`, `2:05`, `12.3 s`, `250 ms`. */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) {
    return "–";
  }
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  if (ms < 60_000) {
    const s = ms / 1000;
    return `${s < 10 ? s.toFixed(1) : Math.floor(s)} s`;
  }
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number): string => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** A clock for running sessions: `0:07`, `1:02:03`. */
export function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number): string => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** Clicks per second from an interval, and back (1 ms … 24 h). */
export function intervalToCps(intervalMs: number): number {
  return intervalMs > 0 ? 1000 / intervalMs : 0;
}

export function cpsToInterval(cps: number): number {
  return cps > 0 ? 1000 / cps : 0;
}

/** `12`, `12.5`, `0.33`: a rate without needless digits. */
export function formatRate(cps: number): string {
  if (!Number.isFinite(cps)) {
    return "–";
  }
  if (cps >= 100) {
    return Math.round(cps).toString();
  }
  if (cps >= 10) {
    return (Math.round(cps * 10) / 10).toString();
  }
  return (Math.round(cps * 100) / 100).toString();
}

export const BUTTON_LABELS: Record<MouseButton, string> = {
  left: "Left",
  right: "Right",
  middle: "Middle",
  back: "Back",
  forward: "Forward",
};

// ------------------------------------------------------------- shortcuts

/** The modifiers of a chord string, in order, and its key code. */
export function splitChord(chord: string): { mods: string[]; key: string } {
  const parts = chord.split("+");
  return { mods: parts.slice(0, -1), key: parts.at(-1) ?? "" };
}

/** `Ctrl + Alt + P` (the key named from the key list). */
export function formatChord(chord: string | null, keys: readonly KeyView[] = []): string {
  if (chord === null || chord === "") {
    return "None";
  }
  const { mods, key } = splitChord(chord);
  const names = mods.map((m) => (m === "Meta" ? META_NAME : m));
  return [...names, keyLabel(key, keys)].join(" + ");
}

export function keyLabel(code: string, keys: readonly KeyView[] = []): string {
  return keys.find((k) => k.code === code)?.label ?? code.replace(/^Key|^Digit/, "");
}

export function keyKind(code: string, keys: readonly KeyView[]): KeyKind | null {
  return keys.find((k) => k.code === code)?.kind ?? null;
}

interface KeyLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

/** The chord a key press makes (`Ctrl+Shift+KeyP`), or `null` while only
 * modifiers are held or the key is unknown. */
export function chordFromEvent(e: KeyLike, keys: readonly KeyView[]): string | null {
  if (MODIFIER_CODES.has(e.code) || !keys.some((k) => k.code === e.code && k.kind !== "modifier")) {
    return null;
  }
  const mods: string[] = [];
  if (e.ctrlKey) {
    mods.push("Ctrl");
  }
  if (e.altKey) {
    mods.push("Alt");
  }
  if (e.shiftKey) {
    mods.push("Shift");
  }
  if (e.metaKey) {
    mods.push("Meta");
  }
  return [...mods, e.code].join("+");
}

/** Why a chord cannot be a shortcut (the rules of `Chord::problem` in
 * keys.rs), or `null`. */
export function chordProblem(chord: string, keys: readonly KeyView[]): string | null {
  const { mods, key } = splitChord(chord);
  const kind = keyKind(key, keys);
  if (kind === null || kind === "modifier") {
    return "Press a key that is not a modifier";
  }
  const has = (m: string): boolean => mods.includes(m);
  const typingSafe = kind === "function" || kind === "media" || kind === "quiet";
  if (!typingSafe && !(has("Ctrl") || has("Alt") || has("Meta"))) {
    return `${formatChord(chord, keys)} alone would stop that key from working everywhere; add Ctrl, Alt or ${META_NAME}`;
  }
  const only = mods.length === 0;
  if (key === "F12" && only) {
    return "F12 alone is reserved by Windows for debuggers";
  }
  const reserved =
    (key === "Delete" && has("Ctrl") && has("Alt")) ||
    (key === "Escape" && (has("Ctrl") || has("Alt"))) ||
    (key === "Tab" && has("Alt")) ||
    (key === "F4" && mods.length === 1 && has("Alt")) ||
    (key === "KeyL" && mods.length === 1 && has("Meta"));
  return reserved ? `${formatChord(chord, keys)} belongs to the system` : null;
}

export const HOTKEY_LABELS: Record<HotkeyAction, string> = {
  "toggle-clicker": "Start or stop the autoclicker",
  "start-clicker": "Start the autoclicker",
  "pause-resume": "Pause or resume",
  "start-recording": "Start recording",
  "stop-recording": "Stop recording",
  "play-macro": "Play the profile's macro",
  "stop-playback": "Stop playback",
  "stop-all": "Emergency stop",
};

// --------------------------------------------------------------- actions

export const ACTION_TYPE_LABELS: Record<ActionType, string> = {
  move: "Move pointer",
  button: "Mouse button",
  click: "Click",
  scroll: "Scroll",
  key: "Key",
  keys: "Shortcut",
  wait: "Wait",
  pause: "Pause point",
};

/** One line describing an action (the live list and the editor). */
export function describeAction(a: ActionKind & { delayMs?: number }, keys: readonly KeyView[] = []): string {
  switch (a.type) {
    case "move":
      return `Move to ${a.x}, ${a.y}`;
    case "button":
      return `${BUTTON_LABELS[a.button]} button ${a.state} at ${a.x}, ${a.y}`;
    case "click": {
      const what = `${BUTTON_LABELS[a.button]} ${a.count === 2 ? "double-click" : "click"}`;
      return a.x === null || a.y === null ? `${what} where the pointer is` : `${what} at ${a.x}, ${a.y}`;
    }
    case "scroll": {
      const notches = Math.max(1, Math.round(Math.abs(a.dy !== 0 ? a.dy : a.dx) / 120));
      const dir = a.dy > 0 ? "up" : a.dy < 0 ? "down" : a.dx > 0 ? "right" : "left";
      return `Scroll ${dir}${notches > 1 ? ` ×${notches}` : ""}`;
    }
    case "key":
      return `${keyLabel(a.key, keys)} ${a.state}`;
    case "keys":
      return `Press ${formatChord(a.chord, keys)}`;
    case "wait":
      return `Wait ${formatDuration(a.delayMs ?? 0)}`;
    case "pause":
      return "Pause until resumed";
  }
}

/** Each action's time from the start of one pass (ms, at normal speed). */
export function actionTimes(actions: readonly Action[]): number[] {
  let t = 0;
  return actions.map((a) => {
    t += a.delayMs;
    return t;
  });
}

export function totalDuration(actions: readonly Action[]): number {
  return actions.reduce((sum, a) => sum + a.delayMs, 0);
}

/** A new action of `type` with sensible defaults, near `at`. */
export function newAction(type: ActionType, at: { x: number; y: number }, delayMs = 100): Action {
  switch (type) {
    case "move":
      return { delayMs, type, x: at.x, y: at.y };
    case "button":
      return { delayMs, type, button: "left", state: "down", x: at.x, y: at.y };
    case "click":
      return { delayMs, type, button: "left", count: 1, x: at.x, y: at.y };
    case "scroll":
      return { delayMs, type, dx: 0, dy: -120, x: null, y: null };
    case "key":
      return { delayMs, type, key: "KeyA", state: "down" };
    case "keys":
      return { delayMs, type, chord: "Ctrl+KeyS" };
    case "wait":
      return { delayMs: Math.max(delayMs, 1000), type };
    case "pause":
      return { delayMs, type };
  }
}

/** The last position an action before `index` put the pointer at. */
export function positionBefore(actions: readonly Action[], index: number): { x: number; y: number } {
  for (let i = Math.min(index, actions.length) - 1; i >= 0; i--) {
    const a = actions[i];
    if (a && "x" in a && a.x !== null && a.y !== null) {
      return { x: a.x, y: a.y };
    }
  }
  return { x: 0, y: 0 };
}

// --------------------------------------------------------------- summaries

/** "Left click · 10 per second · where the pointer is · until stopped". */
export function clickerSummary(c: ClickerConfig): string {
  const button = `${BUTTON_LABELS[c.button]} ${c.clickType === "double" ? "double-click" : "click"}`;
  const mean = c.random.mode === "range" ? (c.random.minMs + c.random.maxMs) / 2 : c.intervalMs;
  const rate =
    c.rateUnit === "cps" ? `${formatRate(intervalToCps(mean))} per second` : `every ${formatDuration(mean)}`;
  const where =
    c.target.mode === "cursor"
      ? "where the pointer is"
      : c.target.mode === "area"
        ? "in an area"
        : c.target.points.length === 1
          ? `at ${c.target.points[0]?.x ?? 0}, ${c.target.points[0]?.y ?? 0}`
          : `at ${c.target.points.length} positions`;
  const until =
    c.maxClicks !== null
      ? `${formatCount(c.maxClicks)} clicks`
      : c.maxDurationMs !== null
        ? `for ${formatDuration(c.maxDurationMs)}`
        : "until stopped";
  return [button, rate + (c.random.mode === "off" ? "" : ", varied"), where, until].join(" · ");
}

/** "Once", "3 times", "Until stopped", with speed when not 1×. */
export function playbackSummary(p: PlaybackSettings): string {
  const reps = p.repeat === null ? "Until stopped" : p.repeat === 1 ? "Once" : `${formatCount(p.repeat)} times`;
  const parts = [reps];
  if (p.speed !== 1) {
    parts.push(`${p.speed}× speed`);
  }
  if (p.stopAfterMs !== null) {
    parts.push(`at most ${formatDuration(p.stopAfterMs)}`);
  }
  if (p.positioning !== "absolute") {
    parts.push(p.positioning === "cursor" ? "relative to the pointer" : "relative to the window");
  }
  return parts.join(" · ");
}

/** Client-side check of a clicker configuration, for immediate feedback
 * (the rules of `ClickerConfig::validate`). */
export function clickerProblem(c: ClickerConfig): string | null {
  const ms = (v: number): boolean => Number.isFinite(v) && v >= 1 && v <= 86_400_000;
  if (!ms(c.intervalMs)) {
    return "The interval must be between 1 ms and 24 hours (at most 1,000 clicks per second).";
  }
  if (c.random.mode === "range") {
    if (!ms(c.random.minMs) || !ms(c.random.maxMs)) {
      return "Random intervals must be between 1 ms and 24 hours.";
    }
    if (c.random.minMs > c.random.maxMs) {
      return "The shortest random interval is longer than the longest one.";
    }
  }
  if (c.random.mode === "variation" && (c.random.variationPercent < 1 || c.random.variationPercent > 90)) {
    return "The random variation must be between 1 % and 90 %.";
  }
  if (c.maxClicks !== null && (c.maxClicks < 1 || c.maxClicks > 100_000_000)) {
    return "The number of clicks must be between 1 and 100,000,000.";
  }
  const shortest =
    c.random.mode === "range"
      ? c.random.minMs
      : c.random.mode === "variation"
        ? c.intervalMs * (1 - c.random.variationPercent / 100)
        : c.intervalMs;
  const perClick = c.clickType === "double" ? 2 * c.holdMs + c.doubleGapMs : c.holdMs;
  if (perClick >= shortest) {
    return `A click takes ${perClick} ms (hold time${c.clickType === "double" ? " and double-click gap" : ""}), longer than the shortest interval.`;
  }
  if (c.target.mode === "points" && c.target.points.length === 0) {
    return "Add at least one click position.";
  }
  if (c.target.mode === "area" && c.target.area === null) {
    return "Set the click area.";
  }
  return null;
}
