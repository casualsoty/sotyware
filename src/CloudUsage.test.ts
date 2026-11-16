// The bucket view's "Usage & costs" tab (ADR-041) and its helpers, headless
// with Tauri's IPC mocks: what the page asks Rust, and that it shows usage
// and estimates exactly as Rust returns them (it never computes a price).
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AccountInfo } from "./cloud";
import CloudUsage from "./CloudUsage.svelte";
import { formatMoney, parseUsageEvent, parseUsageOverview, storedBytes, type Measurement } from "./cloudUsage";
import { formatAgo, formatDecimalSize } from "./format";
import { DOWNLOADS_KEY, loadDownloads, saveDownloads } from "./usagePrefs";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let channels: FakeChannel[];
let overview: Record<string, unknown>;
let app: ReturnType<typeof mount> | undefined;

const ACCOUNT: AccountInfo = {
  id: "b2-photos",
  label: "Photos",
  provider: "backblaze-b2",
  providerName: "Backblaze B2",
  region: "eu-central-003",
  endpoint: null,
  endpointUrl: "https://s3.eu-central-003.backblazeb2.com",
  bucket: "soty-photos",
  configError: null,
  lastCheck: null,
};

function measurement(over: Partial<Measurement> = {}): Measurement {
  return {
    account: "b2-photos",
    bucket: "soty-photos",
    measuredMs: Date.now() - 5 * 60_000,
    durationMs: 1500,
    objects: 1234,
    bytes: 150_000_000_000,
    versionsListed: true,
    noncurrentVersions: 10,
    noncurrentBytes: 50_000_000_000,
    deleteMarkers: 3,
    incompleteListed: true,
    incompleteUploads: 1,
    incompleteBytes: 10_000_000_000,
    incompleteUnmeasured: 0,
    smallObjects: 5,
    smallBytes: 1000,
    complete: true,
    requests: 4,
    notes: [],
    ...over,
  };
}

function estimate(): Record<string, unknown> {
  return {
    currency: "USD",
    period: "30 days",
    total: 1.39,
    components: [
      { id: "storage", label: "Storage", amount: 1.39, detail: "210 GB stored, the first 10.0 GB free, at USD 6.95 per TB for 30 days." },
      { id: "egress", label: "Downloads (egress)", amount: 0, detail: "0 bytes assumed for 30 days; 630 GB free (3 times the stored data), then USD 0.01 per GB." },
      { id: "transactions", label: "API calls", amount: 0, detail: "Class A, B and C API calls are free." },
    ],
    assumptions: ["Prices of Backblaze B2, pay as you go as published on 2026-11-15 (https://www.backblaze.com/cloud-storage/pricing), without taxes."],
  };
}

function group(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    key: "backblaze-b2/eu-central-003",
    provider: "backblaze-b2",
    providerName: "Backblaze B2",
    region: "eu-central-003",
    accounts: ["b2-photos"],
    measured: 1,
    storedBytes: 210_000_000_000,
    objects: 1234,
    downloadsGb: 0,
    estimate: estimate(),
    pricing: {
      name: "Backblaze B2, pay as you go",
      currency: "USD",
      checkedOn: "2026-11-15",
      sources: ["https://www.backblaze.com/cloud-storage/pricing"],
      notes: ["Every B2 bucket keeps old versions."],
      includedStorageBytes: null,
      freeStorageBytes: 10_000_000_000,
    },
    noEstimate: null,
    scopeNote: "Assumes the Backblaze B2 buckets in region eu-central-003 shown here belong to one Backblaze B2 account.",
    ...over,
  };
}

function overviewWith(latest: Measurement | null, history: unknown[] = []): Record<string, unknown> {
  return {
    accounts: [
      {
        account: "b2-photos",
        label: "Photos",
        provider: "backblaze-b2",
        providerName: "Backblaze B2",
        region: "eu-central-003",
        bucket: "soty-photos",
        group: "backblaze-b2/eu-central-003",
        latest,
        history,
      },
    ],
    groups: [latest === null ? group({ measured: 0, estimate: null, noEstimate: "Measure a bucket to see an estimate." }) : group()],
    storedBytes: latest === null ? 0 : storedBytes(latest),
    objects: latest?.objects ?? 0,
    measured: latest === null ? 0 : 1,
    costs: latest === null ? [] : [{ currency: "USD", total: 1.39 }],
    running: null,
    storageError: null,
  };
}

beforeEach(() => {
  calls = [];
  channels = [];
  overview = overviewWith(null);
  localStorage.clear();
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    switch (cmd) {
      case "storage_usage_overview":
        return overview;
      case "measure_storage_usage":
        channels.push(p["onEvent"] as FakeChannel);
        return "usage-1";
      case "cancel_storage_usage":
        return "cancelling";
      default:
        return null;
    }
  });
});

afterEach(() => {
  if (app) {
    unmount(app);
    app = undefined;
  }
  clearMocks();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  for (let i = 0; i < 4; i++) {
    await new Promise((r) => setTimeout(r, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudUsage, { target, props: { account: ACCOUNT } });
  flushSync();
  await settle();
  return target;
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === name);
  if (!found) {
    throw new Error(`no button "${name}" in ${root instanceof Element ? root.innerHTML : ""}`);
  }
  return found;
}

describe("the Usage & costs tab", () => {
  it("before any measurement says why and how sotyware measures, and asks only for the overview", async () => {
    const root = await render();
    expect(calls.map((c) => c.cmd)).toEqual(["storage_usage_overview"]);
    expect(calls[0]!.payload["downloads"]).toEqual({});
    expect(text(root)).toContain("No measurement for Photos yet.");
    expect(text(root)).toContain("Backblaze B2 reports no bucket size through its API");
    expect(button(root, "Measure now").disabled).toBe(false);
    expect(root.querySelector(".estimate")).toBeNull();
  });

  it("measures with progress, then shows the new figures", async () => {
    const root = await render();
    button(root, "Measure now").click();
    await settle();
    const start = calls.find((c) => c.cmd === "measure_storage_usage")!;
    expect(start.payload["account"]).toBe("b2-photos");
    expect(button(root, "Cancel")).toBeTruthy();
    const ch = channels[0]!;
    ch.onmessage({ task: "usage-1", seq: 1, type: "progress", progress: { phase: "versions", entries: 2000, bytes: 5_000_000_000, requests: 2 } });
    await settle();
    expect(text(root.querySelector(".progress-line"))).toContain("Listing every file version: 2,000 found, 5.00 GB so far (2 requests)");
    // A stale or repeated event changes nothing.
    ch.onmessage({ task: "usage-1", seq: 1, type: "progress", progress: { phase: "versions", entries: 1, bytes: 1, requests: 1 } });
    ch.onmessage({ task: "other", seq: 2, type: "progress", progress: { phase: "versions", entries: 9, bytes: 9, requests: 9 } });
    await settle();
    expect(text(root.querySelector(".progress-line"))).toContain("2,000 found");

    const m = measurement();
    overview = overviewWith(m, [{ measuredMs: m.measuredMs, storedBytes: storedBytes(m), objects: m.objects, complete: true }]);
    ch.onmessage({ task: "usage-1", seq: Number.MAX_SAFE_INTEGER, type: "finished", measurement: m, error: null, cancelled: false, saveError: null });
    await settle();
    expect(root.querySelector(".progress-line")).toBeNull();
    expect(calls.filter((c) => c.cmd === "storage_usage_overview")).toHaveLength(2);
    expect(text(root)).toContain("Measured 5 minutes ago");
    expect(button(root, "Measure again")).toBeTruthy();
  });

  it("shows what is stored: tiles, the parts with their figures, and notes", async () => {
    overview = overviewWith(measurement({ notes: ["The stored parts of 1 unfinished upload(s) were not measured."] }));
    const root = await render();
    const tiles = [...root.querySelectorAll(".tile")].map(text);
    expect(tiles[0]).toBe("Stored 210 GB everything kept in the bucket");
    expect(tiles[1]).toBe("Files 1,234 10 older versions");
    expect(tiles[2]).toContain(formatMoney(1.39, "USD"));
    expect(tiles[2]).toContain("estimate");
    const parts = [...root.querySelectorAll(".parts li")].map((li) => [li.getAttribute("data-part"), text(li)]);
    expect(parts).toEqual([
      ["current", "Current files 1,234 150 GB"],
      ["versions", "Older versions and deleted files 10 50.0 GB"],
      ["uploads", "Unfinished uploads 1 10.0 GB"],
    ]);
    // The bar names every part for assistive technology, never by colour alone.
    expect(root.querySelector(".segmented")?.getAttribute("aria-label")).toBe(
      "Current files: 150 GB, Older versions and deleted files: 50.0 GB, Unfinished uploads: 10.0 GB",
    );
    expect(text(root)).toContain("3 delete markers (they store nothing)");
    expect(text(root)).toContain("Backblaze keeps older versions and deleted (hidden) files");
    expect(text(root)).toContain("The stored parts of 1 unfinished upload(s) were not measured.");
  });

  it("keeps the tiles' figures where WebKitGTK exposes them to assistive technology", async () => {
    // Found 2026-11-16 by `cloud-ui.sh usage`: WebKitGTK leaves the text of a
    // span that is a flex item (a tile is a flex column) out of the
    // accessibility tree, so "Stored", "210 GB" and the cost were never read.
    overview = overviewWith(measurement());
    const root = await render();
    const tiles = [...root.querySelectorAll(".tile")];
    expect(tiles).toHaveLength(3);
    for (const tile of tiles) {
      expect([...tile.children].map((c) => c.tagName)).toEqual(["P", "P", "P"]);
    }
  });

  it("labels the cost as an estimate with its parts, assumptions and source, never as a bill", async () => {
    overview = overviewWith(measurement());
    const root = await render();
    const card = root.querySelector(".estimate")!;
    expect(text(card)).toContain("An estimate from Backblaze B2's published prices and sotyware's measurement, not a bill.");
    expect(text(card)).toContain("Backblaze B2 offers no API for billing data");
    const rows = [...card.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-component"));
    expect(rows).toEqual(["storage", "egress", "transactions"]);
    expect(text(card.querySelector("tfoot th"))).toBe("Total for 30 days, without taxes");
    expect(text(card.querySelector("tfoot td"))).toBe(formatMoney(1.39, "USD"));
    expect(text(card.querySelector(".assumptions"))).toContain("as published on 2026-11-15");
    expect(text(card.querySelector(".assumptions"))).toContain("Price list: Backblaze B2, pay as you go, checked 2026-11-15.");
  });

  it("sends the downloads the user expects, remembered on this computer", async () => {
    overview = overviewWith(measurement());
    const root = await render();
    const input = root.querySelector<HTMLInputElement>(".downloads input")!;
    input.value = "700";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 300));
    await settle();
    const last = calls.filter((c) => c.cmd === "storage_usage_overview").at(-1)!;
    expect(last.payload["downloads"]).toEqual({ "backblaze-b2/eu-central-003": 700 });
    expect(JSON.parse(localStorage.getItem(DOWNLOADS_KEY)!)).toEqual({ "backblaze-b2/eu-central-003": 700 });
  });

  it("shows the included storage of a base price as a meter (Hetzner)", async () => {
    const m = measurement({ bytes: 300_000_000_000, noncurrentBytes: 0, incompleteBytes: 0 });
    overview = overviewWith(m);
    (overview["groups"] as Record<string, unknown>[])[0] = group({
      storedBytes: 300_000_000_000,
      pricing: { ...(group()["pricing"] as object), includedStorageBytes: 1_000_000_000_000, freeStorageBytes: 0 },
    });
    const root = await render();
    const meter = root.querySelector('[role="meter"]')!;
    expect(meter.getAttribute("aria-valuetext")).toBe("300 GB of 1.00 TB");
    expect(meter.getAttribute("aria-valuenow")).toBe("300000000000");
  });

  it("draws the history from sotyware's own measurements, with a table", async () => {
    const m = measurement();
    const day = 86_400_000;
    overview = overviewWith(m, [
      { measuredMs: m.measuredMs - 2 * day, storedBytes: 100_000_000_000, objects: 900, complete: true },
      { measuredMs: m.measuredMs - day, storedBytes: 150_000_000_000, objects: 1000, complete: false },
      { measuredMs: m.measuredMs, storedBytes: 210_000_000_000, objects: 1234, complete: true },
    ]);
    const root = await render();
    const svg = root.querySelector("svg.history")!;
    expect(svg.getAttribute("aria-label")).toBe("Stored data in 3 measurements, from 100 GB to 210 GB");
    expect(svg.querySelectorAll("circle")).toHaveLength(3);
    const rows = [...root.querySelectorAll(".history-table tbody tr")].map((r) => text(r.children[1]));
    expect(rows).toEqual(["210 GB", "150 GB+", "100 GB"]);
    expect(text(root)).toContain("sotyware's own measurements of this bucket, not the provider's records.");
  });

  it("reports a failed or cancelled measurement and a refused start", async () => {
    const root = await render();
    button(root, "Measure now").click();
    await settle();
    channels[0]!.onmessage({
      task: "usage-1",
      seq: Number.MAX_SAFE_INTEGER,
      type: "finished",
      measurement: null,
      error: { kind: "access-denied", message: "the provider does not let this key list the bucket's contents" },
      cancelled: false,
      saveError: null,
    });
    await settle();
    expect(text(root.querySelector('[role="alert"]'))).toBe("The measurement failed: the provider does not let this key list the bucket's contents.");

    button(root, "Measure now").click();
    await settle();
    button(root, "Cancel").click();
    await settle();
    expect(calls.find((c) => c.cmd === "cancel_storage_usage")!.payload["task"]).toBe("usage-1");
    channels[1]!.onmessage({ task: "usage-1", seq: Number.MAX_SAFE_INTEGER, type: "finished", measurement: null, error: null, cancelled: true, saveError: null });
    await settle();
    expect(text(root.querySelector('[role="status"]'))).toBe("Measurement cancelled; nothing was kept.");
  });

  it("does not start while another bucket is being measured", async () => {
    overview = { ...overviewWith(null), running: { task: "usage-7", account: "other", startedMs: 1 } };
    const root = await render();
    expect(button(root, "Measure now").disabled).toBe(true);
    expect(text(root)).toContain("Another bucket is being measured; one measurement runs at a time.");
  });
});

describe("usage responses are checked", () => {
  it("accepts what Rust sends and refuses anything else", () => {
    const o = overviewWith(measurement());
    expect(parseUsageOverview(o).groups[0]!.estimate!.total).toBe(1.39);
    expect(() => parseUsageOverview({ ...o, accounts: null })).toThrow(/^invalid usage overview/);
    expect(() => parseUsageOverview({ ...o, storedBytes: "1" })).toThrow(/^invalid usage overview/);
    expect(() => parseUsageEvent({ task: "t", seq: 1, type: "other" })).toThrow(/^invalid usage event/);
    expect(parseUsageEvent({ task: "t", seq: 1, type: "progress", progress: { phase: "uploads", entries: 1, bytes: 2, requests: 3 } })).toEqual({
      task: "t",
      seq: 1,
      type: "progress",
      progress: { phase: "uploads", entries: 1, bytes: 2, requests: 3 },
    });
  });
});

describe("formatting and the downloads preference", () => {
  it("writes sizes in decimal units and ages in words", () => {
    expect(formatDecimalSize(999)).toBe("999 bytes");
    expect(formatDecimalSize(64_000)).toBe("64.0 kB");
    expect(formatDecimalSize(1_234_567_890)).toBe("1.23 GB");
    expect(formatDecimalSize(210_000_000_000)).toBe("210 GB");
    const now = 1_000_000_000_000;
    expect(formatAgo(now - 10_000, now)).toBe("just now");
    expect(formatAgo(now - 60_000, now)).toBe("1 minute ago");
    expect(formatAgo(now - 3 * 3_600_000, now)).toBe("3 hours ago");
    expect(formatAgo(now - 2 * 86_400_000, now)).toBe("2 days ago");
  });

  it("keeps only sensible download assumptions, and survives refused storage", () => {
    const store = new Map<string, string>();
    const fake = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    expect(loadDownloads(fake)).toEqual({});
    expect(saveDownloads("hetzner", 500, fake)).toBe(true);
    expect(saveDownloads("b2/eu", 0, fake)).toBe(true);
    expect(loadDownloads(fake)).toEqual({ hetzner: 500 });
    store.set(DOWNLOADS_KEY, JSON.stringify({ a: -1, b: "x", c: 3 }));
    expect(loadDownloads(fake)).toEqual({ c: 3 });
    store.set(DOWNLOADS_KEY, "not json");
    expect(loadDownloads(fake)).toEqual({});
    const refusing = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    };
    expect(loadDownloads(refusing)).toEqual({});
    expect(saveDownloads("x", 1, refusing)).toBe(false);
    expect(saveDownloads("x", 1, null)).toBe(false);
  });
});
