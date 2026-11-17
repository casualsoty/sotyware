// The Autoclicker page's pure helpers (ADR-050): durations, rates,
// shortcuts from key presses and their rules (the same as Rust's), action
// descriptions, timeline times and summaries.
import { describe, expect, it } from "vitest";
import type { Action, ClickerConfig, KeyView } from "./automation";
import {
  actionTimes,
  chordFromEvent,
  chordProblem,
  clickerProblem,
  clickerSummary,
  cpsToInterval,
  describeAction,
  formatChord,
  formatClock,
  formatDuration,
  formatRate,
  intervalToCps,
  newAction,
  playbackSummary,
  positionBefore,
  totalDuration,
} from "./automationFormat";

const KEYS: KeyView[] = [
  { code: "KeyA", label: "A", kind: "layout" },
  { code: "KeyP", label: "P", kind: "layout" },
  { code: "KeyS", label: "S", kind: "layout" },
  { code: "Digit1", label: "1", kind: "layout" },
  { code: "Space", label: "Space", kind: "other" },
  { code: "F6", label: "F6", kind: "function" },
  { code: "F12", label: "F12", kind: "function" },
  { code: "F4", label: "F4", kind: "function" },
  { code: "Tab", label: "Tab", kind: "other" },
  { code: "Delete", label: "Delete", kind: "other" },
  { code: "Escape", label: "Esc", kind: "other" },
  { code: "Pause", label: "Pause", kind: "quiet" },
  { code: "MediaPlayPause", label: "Play/Pause", kind: "media" },
  { code: "ShiftLeft", label: "Left Shift", kind: "modifier" },
  { code: "ControlLeft", label: "Left Ctrl", kind: "modifier" },
];

const press = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

function clicker(over: Partial<ClickerConfig> = {}): ClickerConfig {
  return {
    button: "left",
    clickType: "single",
    intervalMs: 100,
    rateUnit: "cps",
    random: { mode: "off", minMs: 80, maxMs: 120, variationPercent: 20 },
    maxClicks: null,
    maxDurationMs: null,
    startDelayMs: 0,
    batch: null,
    target: { mode: "cursor", points: [], order: "sequential", area: null, spread: 0 },
    holdMs: 0,
    doubleGapMs: 50,
    advanced: false,
    ...over,
  };
}

describe("durations and rates", () => {
  it("reads naturally at every scale", () => {
    expect(formatDuration(250)).toBe("250 ms");
    expect(formatDuration(3_500)).toBe("3.5 s");
    expect(formatDuration(42_000)).toBe("42 s");
    expect(formatDuration(125_000)).toBe("2:05");
    expect(formatDuration(3_723_000)).toBe("1:02:03");
    expect(formatClock(7_900)).toBe("0:07");
    expect(formatClock(3_723_000)).toBe("1:02:03");
  });

  it("turns rates and intervals into each other", () => {
    expect(intervalToCps(100)).toBe(10);
    expect(cpsToInterval(20)).toBe(50);
    expect(formatRate(12.345)).toBe("12.3");
    expect(formatRate(0.333)).toBe("0.33");
    expect(formatRate(250.4)).toBe("250");
  });
});

describe("shortcuts", () => {
  it("are read from a key press once a key that is not a modifier comes", () => {
    expect(chordFromEvent(press("ControlLeft", { ctrlKey: true }), KEYS)).toBeNull();
    expect(chordFromEvent(press("KeyP", { ctrlKey: true, altKey: true }), KEYS)).toBe("Ctrl+Alt+KeyP");
    expect(chordFromEvent(press("F6"), KEYS)).toBe("F6");
    expect(chordFromEvent(press("KeyP", { shiftKey: true, metaKey: true }), KEYS)).toBe("Shift+Meta+KeyP");
    expect(chordFromEvent(press("IntlYen"), KEYS)).toBeNull();
  });

  it("follow Rust's rules: never take a key from typing or from the system", () => {
    expect(chordProblem("KeyA", KEYS)).toContain("alone would stop that key");
    expect(chordProblem("Shift+KeyA", KEYS)).not.toBeNull();
    expect(chordProblem("Space", KEYS)).not.toBeNull();
    expect(chordProblem("Ctrl+KeyA", KEYS)).toBeNull();
    expect(chordProblem("F6", KEYS)).toBeNull();
    expect(chordProblem("Pause", KEYS)).toBeNull();
    expect(chordProblem("MediaPlayPause", KEYS)).toBeNull();
    expect(chordProblem("F12", KEYS)).toContain("debuggers");
    expect(chordProblem("Ctrl+F12", KEYS)).toBeNull();
    expect(chordProblem("Ctrl+Alt+Delete", KEYS)).toContain("belongs to the system");
    expect(chordProblem("Alt+Tab", KEYS)).not.toBeNull();
    expect(chordProblem("Alt+F4", KEYS)).not.toBeNull();
    expect(chordProblem("Ctrl+Escape", KEYS)).not.toBeNull();
  });

  it("are shown with the keys' names", () => {
    expect(formatChord("Ctrl+Alt+KeyP", KEYS)).toBe("Ctrl + Alt + P");
    expect(formatChord(null, KEYS)).toBe("None");
    expect(formatChord("Shift+F6", KEYS)).toBe("Shift + F6");
  });
});

describe("actions", () => {
  const actions: Action[] = [
    { delayMs: 0, type: "move", x: 10, y: 20 },
    { delayMs: 50, type: "button", button: "left", state: "down", x: 10, y: 20 },
    { delayMs: 80, type: "button", button: "left", state: "up", x: 10, y: 20 },
    { delayMs: 500, type: "keys", chord: "Ctrl+KeyS" },
    { delayMs: 1000, type: "wait" },
    { delayMs: 10, type: "scroll", dx: 0, dy: -360, x: null, y: null },
    { delayMs: 0, type: "click", button: "right", count: 2, x: null, y: null },
    { delayMs: 0, type: "key", key: "KeyA", state: "down" },
    { delayMs: 0, type: "pause" },
  ];

  it("are described in one line", () => {
    expect(actions.map((a) => describeAction(a, KEYS))).toEqual([
      "Move to 10, 20",
      "Left button down at 10, 20",
      "Left button up at 10, 20",
      "Press Ctrl + S",
      "Wait 1.0 s",
      "Scroll down ×3",
      "Right double-click where the pointer is",
      "A down",
      "Pause until resumed",
    ]);
  });

  it("have their times on the timeline", () => {
    expect(actionTimes(actions)).toEqual([0, 50, 130, 630, 1630, 1640, 1640, 1640, 1640]);
    expect(totalDuration(actions)).toBe(1640);
  });

  it("new ones start where the pointer last was", () => {
    expect(positionBefore(actions, 3)).toEqual({ x: 10, y: 20 });
    expect(positionBefore(actions, 0)).toEqual({ x: 0, y: 0 });
    expect(newAction("click", { x: 5, y: 6 })).toEqual({ delayMs: 100, type: "click", button: "left", count: 1, x: 5, y: 6 });
    expect(newAction("wait", { x: 0, y: 0 }).delayMs).toBe(1000);
  });
});

describe("summaries and checks", () => {
  it("say what the autoclicker does", () => {
    expect(clickerSummary(clicker())).toBe("Left click · 10 per second · where the pointer is · until stopped");
    expect(
      clickerSummary(
        clicker({
          clickType: "double",
          rateUnit: "interval",
          intervalMs: 2_000,
          maxClicks: 50,
          target: { mode: "points", points: [{ x: 3, y: 4 }], order: "sequential", area: null, spread: 0 },
        }),
      ),
    ).toBe("Left double-click · every 2.0 s · at 3, 4 · 50 clicks");
  });

  it("say how a macro plays", () => {
    const base = {
      repeat: 1,
      stopAfterMs: null,
      speed: 1,
      startDelayMs: 0,
      repeatDelayMs: 0,
      jitterPercent: 0,
      positioning: "absolute" as const,
      scaleToScreen: true,
      section: null,
    };
    expect(playbackSummary(base)).toBe("Once");
    expect(playbackSummary({ ...base, repeat: null, speed: 2, positioning: "cursor" })).toBe(
      "Until stopped · 2× speed · relative to the pointer",
    );
  });

  it("catch what Rust would refuse before saving", () => {
    expect(clickerProblem(clicker())).toBeNull();
    expect(clickerProblem(clicker({ intervalMs: 0.5 }))).toContain("1 ms");
    expect(clickerProblem(clicker({ random: { mode: "range", minMs: 200, maxMs: 100, variationPercent: 20 } }))).toContain(
      "longer than the longest",
    );
    expect(clickerProblem(clicker({ clickType: "double", intervalMs: 50, holdMs: 10, doubleGapMs: 30 }))).toContain(
      "longer than the shortest interval",
    );
    expect(clickerProblem(clicker({ target: { mode: "points", points: [], order: "sequential", area: null, spread: 0 } }))).toContain(
      "position",
    );
  });
});
