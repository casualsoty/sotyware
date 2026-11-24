<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { CloudCommandError, type AccountInfo } from "./cloud";
  import { describeError } from "./cloudMessages";
  import {
    cancelUsage,
    formatMoney,
    measureUsage,
    storedBytes,
    usageOverview,
    type AccountUsage,
    type ProviderGroup,
    type UsageEvent,
    type UsageOverview,
    type UsageProgress,
  } from "./cloudUsage";
  import type { SyncStatus } from "./folderSync";
  import { formatAgo, formatCount, formatDate, formatDateTime, formatDecimalSize } from "./format";
  import Icon from "./Icon.svelte";
  import { loadDownloads, saveDownloads } from "./usagePrefs";

  // A bucket's storage usage and estimated cost (ADR-041,
  // docs/architecture/cloud-storage.md#storage-usage-and-cost-estimates):
  // the "Usage & costs" tab of the bucket view. Rust measures by listing the
  // bucket (reads only) and computes the estimate from published prices;
  // this page only shows what it returns. Nothing here is a bill.

  interface Props {
    account: AccountInfo;
    /** The bucket's folder sync, if it has one (ADR-043): what it keeps on
     * this computer, told apart from what the bucket stores. */
    sync?: SyncStatus | null;
  }

  let { account, sync = null }: Props = $props();

  let overview = $state<UsageOverview | null>(null);
  let loadError = $state<string | null>(null);
  let task = $state<string | null>(null);
  let progress = $state<UsageProgress | null>(null);
  let measureError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let cancelling = $state(false);
  let downloadsText = $state("");
  let destroyed = false;
  let lastSeq = 0;
  let reloadTimer: ReturnType<typeof setTimeout> | undefined;

  const mine = $derived<AccountUsage | null>(overview?.accounts.find((a) => a.account === account.id) ?? null);
  const latest = $derived(mine?.latest ?? null);
  const group = $derived<ProviderGroup | null>(overview?.groups.find((g) => g.key === mine?.group) ?? null);
  const otherRunning = $derived(
    overview?.running !== null && overview?.running !== undefined && overview.running.account !== account.id && task === null,
  );
  const runningHere = $derived(task !== null || (overview?.running?.account === account.id && task === null));
  const name = $derived(account.providerName ?? account.provider);

  /** Busy means another measurement here, not a download. */
  function describe(e: unknown): string {
    return e instanceof CloudCommandError && e.kind === "busy" ? `${e.message}.` : describeError(e);
  }

  async function load(): Promise<void> {
    try {
      const o = await usageOverview(loadDownloads());
      if (destroyed) {
        return;
      }
      overview = o;
      loadError = null;
      const g = o.groups.find((x) => x.key === o.accounts.find((a) => a.account === account.id)?.group);
      if (g && downloadsText === "") {
        downloadsText = g.downloadsGb > 0 ? String(g.downloadsGb) : "";
      }
    } catch (e) {
      loadError = `Could not read the storage usage: ${describeError(e)}`;
    }
  }

  function onEvent(e: UsageEvent): void {
    if (destroyed || e.task !== task || e.seq <= lastSeq) {
      return;
    }
    lastSeq = e.seq;
    if (e.type === "progress") {
      progress = e.progress;
      return;
    }
    task = null;
    progress = null;
    cancelling = false;
    if (e.cancelled) {
      notice = "Measurement cancelled; nothing was kept.";
    } else if (e.error !== null) {
      measureError = `The measurement failed: ${e.error.message}.`;
    } else if (e.saveError !== null) {
      notice = `Measured, but the result could not be kept for later: ${e.saveError}`;
    }
    void load();
  }

  async function measure(): Promise<void> {
    measureError = null;
    notice = null;
    lastSeq = 0;
    progress = null;
    try {
      task = "starting";
      task = await measureUsage(account.id, onEvent);
    } catch (e) {
      task = null;
      measureError = `The measurement could not start: ${describe(e)}`;
    }
  }

  async function cancel(): Promise<void> {
    const t = task ?? overview?.running?.task ?? null;
    if (t === null || t === "starting") {
      return;
    }
    cancelling = true;
    try {
      const outcome = await cancelUsage(t);
      if (outcome === "not-active") {
        cancelling = false;
        void load();
      }
    } catch (e) {
      cancelling = false;
      measureError = `Could not cancel: ${describeError(e)}`;
    }
  }

  function changeDownloads(): void {
    const key = group?.key;
    if (key === undefined) {
      return;
    }
    const gb = Number(downloadsText.replace(",", "."));
    saveDownloads(key, downloadsText.trim() === "" || !Number.isFinite(gb) || gb < 0 ? 0 : gb);
    clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => void load(), 250);
  }

  const PHASES: Record<string, string> = {
    versions: "Listing every file version",
    objects: "Listing the files",
    uploads: "Listing unfinished uploads",
  };

  function when(ms: number): string {
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? "" : formatDateTime(d);
  }

  /** The parts of what is stored, for the segmented bar and its list. */
  const parts = $derived.by(() => {
    if (latest === null) {
      return [];
    }
    return [
      { id: "current", label: "Current files", bytes: latest.bytes, count: latest.objects, swatch: "var(--chart-1)" },
      {
        id: "versions",
        label: "Older versions and deleted files",
        bytes: latest.noncurrentBytes,
        count: latest.noncurrentVersions,
        swatch: "var(--chart-2)",
      },
      {
        id: "uploads",
        label: "Unfinished uploads",
        bytes: latest.incompleteBytes,
        count: latest.incompleteUploads,
        swatch: "var(--chart-3)",
      },
    ];
  });
  const total = $derived(latest === null ? 0 : storedBytes(latest));

  // The history chart: stored bytes over sotyware's own measurements,
  // drawn at the card's real width so text and marks keep their size.
  let chartWidth = $state(560);
  const H = 132;
  const PAD = { top: 24, right: 14, bottom: 24, left: 14 };
  const W = $derived(Math.max(240, chartWidth));
  const chart = $derived.by(() => {
    const points = mine?.history ?? [];
    if (points.length < 2) {
      return null;
    }
    const xs = points.map((p) => p.measuredMs);
    const ys = points.map((p) => p.storedBytes);
    const [x0, x1] = [Math.min(...xs), Math.max(...xs)];
    const y1 = Math.max(...ys, 1);
    const sx = (x: number): number => PAD.left + ((x - x0) / Math.max(x1 - x0, 1)) * (W - PAD.left - PAD.right);
    const sy = (y: number): number => H - PAD.bottom - (y / y1) * (H - PAD.top - PAD.bottom);
    return {
      line: points.map((p) => `${sx(p.measuredMs).toFixed(1)},${sy(p.storedBytes).toFixed(1)}`).join(" "),
      dots: points.map((p) => ({ x: sx(p.measuredMs), y: sy(p.storedBytes), p })),
      max: y1,
      top: sy(y1),
      first: points[0]!,
      last: points[points.length - 1]!,
      baseline: H - PAD.bottom,
    };
  });

  onMount(() => {
    void load();
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(reloadTimer);
  });
</script>

<section class="usage" aria-labelledby="usage-heading" data-usage={account.id}>
  <header class="usage-head">
    <div>
      <h2 id="usage-heading">Storage usage</h2>
      <p class="muted small-text">
        {#if latest}
          Measured {formatAgo(latest.measuredMs)} ({when(latest.measuredMs)}) in {(latest.durationMs / 1000).toFixed(1)} s with
          {formatCount(latest.requests)} listing request{latest.requests === 1 ? "" : "s"}.
        {:else if overview}
          Not measured yet.
        {:else}
          Loading…
        {/if}
      </p>
    </div>
    <div class="actions">
      {#if runningHere}
        <button type="button" disabled={cancelling} onclick={() => void cancel()}>{cancelling ? "Cancelling…" : "Cancel"}</button>
      {:else}
        <button type="button" class="primary" disabled={otherRunning || overview === null} onclick={() => void measure()}
          ><Icon name="refresh" size={14} />{latest ? "Measure again" : "Measure now"}</button
        >
      {/if}
    </div>
  </header>

  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {/if}
  {#if overview?.storageError}
    <p class="notice warning">Measurements cannot be kept on this computer: {overview.storageError}</p>
  {/if}
  {#if measureError}
    <p class="error" role="alert">{measureError}</p>
  {/if}
  {#if notice}
    <p class="notice" role="status">{notice}</p>
  {/if}
  {#if otherRunning && overview?.running}
    <p class="notice" role="status">Another bucket is being measured; one measurement runs at a time.</p>
  {/if}

  {#if runningHere}
    <div class="progress-line" role="status">
      <span class="spinner" aria-hidden="true"></span>
      {#if progress}
        {PHASES[progress.phase] ?? "Measuring"}: {formatCount(progress.entries)} found, {formatDecimalSize(progress.bytes)} so far
        ({formatCount(progress.requests)} request{progress.requests === 1 ? "" : "s"}).
      {:else}
        Measuring… Every version in the bucket is listed (one request per 1,000; reads only).
      {/if}
    </div>
  {/if}

  {#if overview && latest === null && !runningHere}
    <div class="empty">
      <span aria-hidden="true"><Icon name="info" size={22} /></span>
      <p><strong>No measurement for {account.label} yet.</strong></p>
      <p>
        {name} reports no bucket size through its API, so sotyware lists the bucket to measure it: every file and every
        older version, one request per 1,000 entries. It only reads. A large bucket can take a few minutes.
      </p>
    </div>
  {/if}

  {#if latest}
    <div class="tiles">
      <div class="tile">
        <p class="tile-label">Stored</p>
        <p class="tile-value">{formatDecimalSize(total)}</p>
        <p class="tile-sub">{latest.complete ? "everything kept in the bucket" : "at least (the measurement stopped early)"}</p>
      </div>
      <div class="tile">
        <p class="tile-label">Files</p>
        <p class="tile-value">{formatCount(latest.objects)}</p>
        <p class="tile-sub">{latest.versionsListed ? `${formatCount(latest.noncurrentVersions)} older versions` : "versions not listed"}</p>
      </div>
      <div class="tile">
        <p class="tile-label">Estimated cost, {group?.estimate?.period ?? "30 days"}</p>
        {#if group?.estimate}
          <p class="tile-value">{formatMoney(group.estimate.total, group.estimate.currency)}</p>
          <p class="tile-sub"
            ><span class="badge info">estimate</span>
            {group.accounts.length > 1 ? `for ${group.accounts.length} buckets of this ${name} account` : "without taxes"}</p
          >
        {:else}
          <p class="tile-value none">—</p>
          <p class="tile-sub">{group?.noEstimate ?? ""}</p>
        {/if}
      </div>
    </div>

    <div class="card">
      <h3 class="section-title">What is stored</h3>
      {#if total > 0}
        <div
          class="segmented"
          role="img"
          aria-label={parts.map((p) => `${p.label}: ${formatDecimalSize(p.bytes)}`).join(", ")}
        >
          {#each parts.filter((p) => p.bytes > 0) as p (p.id)}
            <span class="segment" style="flex-grow: {p.bytes}; background: {p.swatch}" title="{p.label}: {formatDecimalSize(p.bytes)}"></span>
          {/each}
        </div>
      {/if}
      <ul class="parts">
        {#each parts as p (p.id)}
          <li data-part={p.id}>
            <span class="swatch" style="background: {p.swatch}" aria-hidden="true"></span>
            <span class="part-label">{p.label}</span>
            <span class="part-count">{formatCount(p.count)}</span>
            <span class="part-bytes">{formatDecimalSize(p.bytes)}</span>
          </li>
        {/each}
      </ul>
      <p class="muted small-text">
        {#if latest.versionsListed}
          {formatCount(latest.deleteMarkers)} delete marker{latest.deleteMarkers === 1 ? "" : "s"} (they store nothing).
          {#if account.provider === "backblaze-b2"}
            Backblaze keeps older versions and deleted (hidden) files in every bucket until a lifecycle rule removes them, and
            bills them as storage.
          {/if}
        {/if}
        Sizes in decimal units (1 GB = 1,000,000,000 bytes), as providers bill them.
      </p>
      {#each latest.notes as n (n)}
        <p class="callout warning small-text">{n}</p>
      {/each}

      {#if group?.pricing?.includedStorageBytes}
        {@const included = group.pricing.includedStorageBytes}
        {@const used = Math.min(group.storedBytes / included, 1)}
        <div class="meter-block">
          <div class="meter-head">
            <span>Included in the base price</span>
            <span class="meter-figure">{formatDecimalSize(group.storedBytes)} of {formatDecimalSize(included)}</span>
          </div>
          <div
            class="meter"
            role="meter"
            aria-label="Storage included in the base price"
            aria-valuemin="0"
            aria-valuemax={included}
            aria-valuenow={Math.min(group.storedBytes, included)}
            aria-valuetext="{formatDecimalSize(group.storedBytes)} of {formatDecimalSize(included)}"
          >
            <span class="meter-fill" class:over={group.storedBytes > included} style="width: {(used * 100).toFixed(1)}%"></span>
          </div>
          <p class="muted small-text">
            Across the {group.measured} measured bucket{group.measured === 1 ? "" : "s"} of this {name} account{group.storedBytes >
            included
              ? "; the storage above it is billed per TB-hour"
              : ""}.
          </p>
        </div>
      {/if}
    </div>

    {#if chart}
      <div class="card">
        <h3 class="section-title">Stored over time</h3>
        <div class="history-frame" bind:clientWidth={chartWidth}>
          <svg
            class="history"
            width={W}
            height={H}
            viewBox="0 0 {W} {H}"
            role="img"
            aria-label="Stored data in {mine?.history.length} measurements, from {formatDecimalSize(chart.first.storedBytes)} to {formatDecimalSize(chart.last.storedBytes)}"
          >
            <line class="grid" x1={PAD.left} x2={W - PAD.right} y1={chart.top} y2={chart.top} />
            <line class="axis" x1={PAD.left} x2={W - PAD.right} y1={chart.baseline} y2={chart.baseline} />
            <polyline class="line" points={chart.line} />
            {#each chart.dots as d (d.p.measuredMs)}
              <circle class="dot-mark" cx={d.x} cy={d.y} r="4"><title>{when(d.p.measuredMs)}: {formatDecimalSize(d.p.storedBytes)}{d.p.complete ? "" : " (at least)"}</title></circle>
            {/each}
            <text class="tick" x={PAD.left} y={chart.top - 7}>{formatDecimalSize(chart.max)}</text>
            <text class="tick" x={PAD.left} y={chart.baseline - 6}>0</text>
            <text class="tick" x={PAD.left} y={H - 6}>{formatDate(new Date(chart.first.measuredMs))}</text>
            <text class="tick end" x={W - PAD.right} y={H - 6}>{formatDate(new Date(chart.last.measuredMs))}</text>
          </svg>
        </div>
        <details>
          <summary class="small-text">Measurements as a table</summary>
          <table class="history-table">
            <thead><tr><th>Measured</th><th class="num">Stored</th><th class="num">Files</th></tr></thead>
            <tbody>
              {#each [...(mine?.history ?? [])].reverse() as h (h.measuredMs)}
                <tr><td>{when(h.measuredMs)}</td><td class="num">{formatDecimalSize(h.storedBytes)}{h.complete ? "" : "+"}</td><td class="num">{formatCount(h.objects)}</td></tr>
              {/each}
            </tbody>
          </table>
        </details>
        <p class="muted small-text">sotyware's own measurements of this bucket, not the provider's records.</p>
      </div>
    {/if}
  {/if}

  {#if group && latest}
    <div class="card estimate" aria-labelledby="estimate-heading">
      <h3 class="section-title" id="estimate-heading">Estimated cost</h3>
      {#if group.estimate}
        {@const e = group.estimate}
        <p class="callout neutral small-text">
          An <strong>estimate</strong> from {name}'s published prices and sotyware's measurement, not a bill.
          {name} offers no API for billing data: your invoice in the {name} console is what counts.
        </p>
        <table class="components">
          <tbody>
            {#each e.components as c (c.id)}
              <tr data-component={c.id}>
                <th scope="row">{c.label}<span class="component-detail">{c.detail}</span></th>
                <td class="num">{formatMoney(c.amount, e.currency)}</td>
              </tr>
            {/each}
          </tbody>
          <tfoot>
            <tr><th scope="row">Total for {e.period}, without taxes</th><td class="num total">{formatMoney(e.total, e.currency)}</td></tr>
          </tfoot>
        </table>
        <label class="field downloads">
          <span>Downloads you expect per 30 days (GB)</span>
          <input
            type="text"
            inputmode="decimal"
            placeholder="0"
            bind:value={downloadsText}
            oninput={changeDownloads}
            aria-describedby="downloads-hint"
          />
          <span class="hint" id="downloads-hint">sotyware cannot measure downloads made by other programs; the estimate uses this figure.</span>
        </label>
        <details class="assumptions">
          <summary class="small-text">How this is estimated</summary>
          <ul class="small-text">
            {#each e.assumptions as a (a)}
              <li>{a}</li>
            {/each}
          </ul>
          {#if group.pricing}
            <p class="small-text muted">Price list: {group.pricing.name}, checked {group.pricing.checkedOn}.</p>
            <ul class="small-text muted">
              {#each group.pricing.notes as note (note)}
                <li>{note}</li>
              {/each}
            </ul>
          {/if}
        </details>
      {:else}
        <p class="muted">{group.noEstimate}</p>
      {/if}
    </div>
  {/if}

  {#if sync}
    <div class="card local-copy" aria-labelledby="local-copy-heading" data-usage-local>
      <h3 class="section-title" id="local-copy-heading">On this computer (folder sync)</h3>
      <p class="small-text muted">
        A copy of the bucket's current files is kept in <span class="mono">{sync.path}</span>. It takes space on this computer,
        not in the bucket, and costs nothing at the provider beyond the downloads that filled it.
      </p>
      <dl class="meta-list">
        <dt>Synced here</dt>
        <dd>{formatDecimalSize(sync.bytes)} in {formatCount(sync.files)} files</dd>
        <dt>Waiting to upload</dt>
        <dd>{formatCount(sync.pendingUploads)} files, {formatDecimalSize(sync.pendingUploadBytes)}</dd>
        <dt>Waiting to download</dt>
        <dd>{formatCount(sync.pendingDownloads)} files, {formatDecimalSize(sync.pendingDownloadBytes)}</dd>
        {#if sync.trashFiles > 0}
          <dt>Sync trash</dt>
          <dd>{formatDecimalSize(sync.trashBytes)} in {formatCount(sync.trashFiles)} files (kept 30 days)</dd>
        {/if}
      </dl>
    </div>
  {/if}
</section>

<style>
  .usage {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: var(--content-w);
  }

  .usage-head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .usage-head h2 {
    margin-bottom: var(--space-1);
  }

  .progress-line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
    gap: var(--space-3);
  }

  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    min-width: 0;
  }

  /* Paragraphs, not spans: WebKitGTK leaves the text of a span that is a
     flex item out of the accessibility tree (frontend.md), so a screen
     reader never read the figures. */
  .tile-label,
  .tile-value,
  .tile-sub {
    margin: 0;
  }

  .tile-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .tile-value {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    line-height: var(--leading-tight);
  }

  .tile-value.none {
    color: var(--text-subtle);
  }

  .tile-sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-subtle);
  }

  /* A thin segmented bar: one whole, its parts named in the list below. */
  .segmented {
    display: flex;
    gap: 2px;
    height: 8px;
    margin: var(--space-1) 0 var(--space-3);
  }

  .segment {
    flex-basis: 0;
    min-width: 3px;
    border-radius: var(--radius-full);
  }

  .parts {
    list-style: none;
    margin: 0 0 var(--space-2);
    padding: 0;
    display: grid;
    gap: var(--space-1);
  }

  .parts li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto 6.5rem;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
  }

  .part-count,
  .part-bytes {
    font-variant-numeric: tabular-nums;
    text-align: right;
    color: var(--text-muted);
  }

  .part-bytes {
    color: var(--text);
  }

  .meter-block {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }

  .meter-head {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-sm);
    margin-bottom: var(--space-1);
  }

  .meter-figure {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }

  .meter {
    display: block;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--chart-track);
    overflow: hidden;
  }

  .meter-fill {
    display: block;
    height: 100%;
    border-radius: var(--radius-full);
    background: var(--chart-1);
  }

  .meter-fill.over {
    background: var(--warning);
  }

  .history-frame {
    width: 100%;
    overflow: hidden;
  }

  .history {
    display: block;
  }

  .history .axis,
  .history .grid {
    stroke: var(--border);
    stroke-width: 1;
  }

  .history .line {
    fill: none;
    stroke: var(--chart-1);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .history .dot-mark {
    fill: var(--chart-1);
    stroke: var(--surface);
    stroke-width: 2;
  }

  .history .tick {
    font-size: 11px;
    fill: var(--text-subtle);
  }

  .history .tick.end {
    text-anchor: end;
  }

  .history-table,
  .components {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .history-table th,
  .history-table td {
    position: static;
    padding: var(--space-1) var(--space-2);
    background: transparent;
    text-align: left;
    border-bottom: 1px solid var(--border);
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  /* Row headers of a figures table, not the file table's sticky column
     headers (base.css). */
  .components th,
  .components td {
    position: static;
    padding: var(--space-2) 0;
    background: transparent;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
    text-align: left;
    font-size: var(--text-sm);
    font-weight: var(--weight-regular);
    color: var(--text);
  }

  /* A row header carries a sentence of detail: it wraps inside the card
     instead of keeping base.css's single line for column headers, which
     pushed the table past the card's edge. */
  .components th {
    white-space: normal;
  }

  .components td.num {
    text-align: right;
    padding-left: var(--space-4);
  }

  .component-detail {
    display: block;
    margin-top: 2px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .components tfoot th,
  .components tfoot td {
    border-bottom: 0;
    font-weight: var(--weight-semibold);
  }

  .downloads {
    margin-top: var(--space-3);
    max-width: 24rem;
  }

  .assumptions ul {
    margin: var(--space-2) 0;
    padding-left: var(--space-5);
  }

  details summary {
    cursor: pointer;
    color: var(--accent-text);
    margin: var(--space-2) 0;
  }
</style>
