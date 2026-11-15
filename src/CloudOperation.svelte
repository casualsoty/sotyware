<script lang="ts">
  import { onDestroy } from "svelte";
  import { CloudCommandError, type AccountInfo } from "./cloud";
  import {
    cancelOperation,
    discardPlan,
    planOperation,
    retryOperation,
    startOperation,
    type ItemReport,
    type OperationEvent,
    type OperationKind,
    type OperationRequest,
    type OperationStarted,
    type OperationSummary,
    type PlanSummary,
  } from "./cloudFiles";
  import { describeOperationError, describeOperationFailure } from "./cloudMessages";
  import { formatSize, shownName } from "./format";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  // Copy, move, rename, delete and restore (ADR-026): Rust plans the
  // operation (reads only), this component shows the plan for review
  // (items, object counts, conflicts, what a delete means on this bucket),
  // starts it only when the user confirms, then shows what the events say.
  // Plans, ETags and version ids stay in Rust; events of another or older
  // operation are dropped (operation id and seq).

  interface Props {
    account: AccountInfo;
    bucket: string;
    /** Something changed in the bucket: refresh what is shown. */
    onchanged: () => void;
    /** Copy or move again with another destination (from the review). */
    onrechoose?: ((request: OperationRequest) => void) | undefined;
    onedit: () => void;
    busy?: boolean;
  }

  let { account, bucket, onchanged, onrechoose, onedit, busy = $bindable(false) }: Props = $props();

  type Phase =
    | { step: "idle" }
    | { step: "planning"; request: OperationRequest }
    | { step: "review"; request: OperationRequest; plan: PlanSummary; policy: "skip" | "rename"; starting: boolean; error: unknown }
    | { step: "running" | "finished"; kind: OperationKind; operation: string; items: ItemReport[]; summary: OperationSummary | null; cancel: boolean; error: unknown; progress: { done: number; total: number } };

  let phase = $state<Phase>({ step: "idle" });
  /** Bumped by every plan request: an answer for an older one is dropped.
   * (Not object identity: `phase` holds a proxy of the request.) */
  let planRequest = 0;
  let planError = $state<unknown>(null);
  let seen = new Map<string, number>();
  let destroyed = false;

  $effect(() => {
    busy = phase.step === "planning" || phase.step === "review" || phase.step === "running";
  });

  const VERBS: Record<OperationKind, string> = {
    copy: "Copy",
    move: "Move",
    rename: "Rename",
    delete: "Delete",
    restore: "Restore",
  };

  /** Plans `request` and shows it for review. */
  export async function run(request: OperationRequest): Promise<void> {
    if (busy) {
      return;
    }
    planError = null;
    const mine = ++planRequest;
    phase = { step: "planning", request };
    try {
      const plan = await planOperation(account.id, bucket, request);
      if (destroyed || phase.step !== "planning" || mine !== planRequest) {
        void discardPlan(plan.plan).catch(() => undefined);
        return;
      }
      // Every item left out (a folder pasted into itself, a file moved
      // into its own folder, items gone since): nothing to review or
      // start, so say why instead of opening an empty "Move 0 items" review.
      if (plan.items.length > 0 && plan.items.every((i) => i.problem !== null)) {
        void discardPlan(plan.plan).catch(() => undefined);
        phase = { step: "idle" };
        planError = new Error(nothingToDo(plan));
        return;
      }
      phase = { step: "review", request, plan, policy: "skip", starting: false, error: null };
    } catch (e) {
      if (!destroyed && phase.step === "planning" && mine === planRequest) {
        phase = { step: "idle" };
        planError = e;
      }
    }
  }

  function closeReview(): void {
    if (phase.step === "review" && !phase.starting) {
      void discardPlan(phase.plan.plan).catch(() => undefined);
      phase = { step: "idle" };
    }
  }

  function rechoose(): void {
    if (phase.step === "review" && onrechoose) {
      const request = phase.request;
      closeReview();
      onrechoose(request);
    }
  }

  function onEvent(event: OperationEvent): void {
    if (destroyed || (phase.step !== "running" && phase.step !== "finished")) {
      return;
    }
    if (event.operation !== phase.operation) {
      return;
    }
    const last = seen.get(event.operation) ?? 0;
    if (event.seq <= last) {
      return;
    }
    seen.set(event.operation, event.seq);
    if (event.type === "item") {
      phase.items[event.item.index] = event.item;
    } else if (event.type === "progress") {
      phase.progress = { done: event.objectsDone, total: event.objectsTotal };
    } else {
      phase.summary = event.summary;
      phase.step = "finished";
      phase.cancel = false;
      onchanged();
    }
  }

  function onInvalid(e: Error): void {
    if (phase.step === "running") {
      phase.error = e;
    }
  }

  function adopt(started: OperationStarted): void {
    if (phase.step === "running" && phase.operation === started.operation) {
      return;
    }
    phase = {
      step: "running",
      kind: started.kind,
      operation: started.operation,
      items: started.items,
      summary: null,
      cancel: false,
      error: null,
      progress: { done: 0, total: started.items.reduce((n, i) => n + i.objectsTotal, 0) },
    };
  }

  // Events may arrive before start resolves: they are kept until then.
  let early: OperationEvent[] = [];

  function buffered(event: OperationEvent): void {
    if (phase.step === "running" || phase.step === "finished") {
      onEvent(event);
    } else {
      early.push(event);
    }
  }

  function flush(): void {
    const pending = early;
    early = [];
    for (const e of pending) {
      onEvent(e);
    }
  }

  async function confirm(): Promise<void> {
    if (phase.step !== "review" || phase.starting) {
      return;
    }
    const review = phase;
    review.starting = true;
    review.error = null;
    early = [];
    try {
      const started = await startOperation(review.plan.plan, review.policy, buffered, onInvalid);
      if (destroyed) {
        void cancelOperation(started.operation).catch(() => undefined);
        return;
      }
      adopt(started);
      flush();
    } catch (e) {
      if (phase.step === "review") {
        phase.starting = false;
        phase.error = e;
      }
    }
  }

  async function cancel(): Promise<void> {
    if (phase.step !== "running" || phase.cancel) {
      return;
    }
    phase.cancel = true;
    try {
      await cancelOperation(phase.operation);
    } catch (e) {
      if (phase.step === "running") {
        phase.cancel = false;
        phase.error = e;
      }
    }
  }

  async function retry(): Promise<void> {
    if (phase.step !== "finished" || phase.summary === null || !phase.summary.retryable) {
      return;
    }
    const previous = phase.operation;
    early = [];
    try {
      const started = await retryOperation(previous, buffered, onInvalid);
      adopt(started);
      flush();
    } catch (e) {
      if (phase.step === "finished") {
        phase.error = e;
      }
    }
  }

  function dismiss(): void {
    if (phase.step === "finished") {
      phase = { step: "idle" };
    }
    planError = null;
  }

  /** Leaving the browser cancels a running operation (as for transfers). */
  onDestroy(() => {
    destroyed = true;
    if (phase.step === "running") {
      void cancelOperation(phase.operation).catch(() => undefined);
    } else if (phase.step === "review") {
      void discardPlan(phase.plan.plan).catch(() => undefined);
    }
  });

  function where(path: string | null): string {
    return path === null || path === "" ? `the root of ${bucket}` : path;
  }

  /** Why a plan whose items were all left out does nothing. */
  function nothingToDo(p: PlanSummary): string {
    const reasons = p.items
      .slice(0, 3)
      .map((i) => `“${shownName(i.name)}”: ${i.problem ?? ""}`)
      .join("; ");
    const more = p.items.length > 3 ? `; and ${p.items.length - 3} more` : "";
    return `Nothing to ${p.kind}. ${reasons}${more}.`;
  }

  function reviewTitle(p: PlanSummary): string {
    const usable = p.items.filter((i) => i.problem === null);
    const what =
      usable.length === 1 && usable[0] !== undefined
        ? `“${shownName(usable[0].name)}”${usable[0].folder ? " and everything in it" : ""}`
        : `${usable.length} items`;
    switch (p.kind) {
      case "delete":
        return `Delete ${what}?`;
      case "copy":
        return `Copy ${what} to ${where(p.destination)}?`;
      case "move":
        return `Move ${what} to ${where(p.destination)}?`;
      case "rename":
        return `Rename ${what}?`;
      case "restore":
        return `Restore ${what}?`;
    }
  }

  function versioningNote(p: PlanSummary): string {
    switch (p.versioning) {
      case "enabled":
        return "This bucket keeps versions: deleted files are hidden behind a delete marker and can be restored from Deleted items until their versions are removed (for example by a lifecycle rule). Hidden versions are still stored and billed.";
      case "suspended":
        return "Versioning is suspended on this bucket: deleted files may be gone for good.";
      case "disabled":
        return "This bucket does not keep versions: deleted files are gone for good and cannot be restored.";
      case "unknown":
        return "sotyware could not find out whether this bucket keeps versions (this key may not ask). Assume deleted files cannot be restored.";
    }
  }

  function moveNote(p: PlanSummary): string {
    return p.kind === "rename"
      ? "Object storage has no rename: every object is copied to its new name and checked, and only then is the original deleted."
      : "Every object is copied and checked first; the originals are deleted only once all copies are verified.";
  }

  function itemLine(i: ItemReport): string {
    const counts = i.folder ? ` · ${i.objectsDone} of ${i.objectsTotal} objects` : "";
    switch (i.state) {
      case "pending":
        return "Waiting";
      case "running":
        return `Working${counts}`;
      case "completed":
        return phaseKindDone(i);
      case "skipped":
        return "Skipped: something with this name already exists there; nothing was replaced";
      case "failed":
        return i.note === "original-kept-copy-incomplete" ? `Failed; the original is complete and was kept${counts}` : `Failed${counts}`;
      case "partial":
        switch (i.note) {
          case "copied-original-kept":
            return "Copied and verified, but the original could not be deleted; it is still there";
          case "original-kept-copy-incomplete":
            return `Not moved: only part was copied, so nothing was deleted${counts}. Copies already made stay in the destination.`;
          case "source-not-empty":
            return `Every reviewed object was handled${counts}, but the original folder holds files added after the review; they were left where they are. Review again to include them.`;
          default:
            return `Only partly done${counts}`;
        }
      case "cancelled":
        return "Cancelled; not started";
    }
  }

  function phaseKindDone(i: ItemReport): string {
    if (phase.step !== "running" && phase.step !== "finished") {
      return "Done";
    }
    switch (phase.kind) {
      case "copy":
        return `Copied${i.destination !== null ? ` to ${i.destination}` : ""}`;
      case "move":
      case "rename":
        return `Moved${i.destination !== null ? ` to ${i.destination}` : ""}`;
      case "delete":
        return i.removed > 0 && i.hidden === 0
          ? "Deleted for good"
          : i.removed === 0
            ? "Deleted (restorable from Deleted items)"
            : `Deleted: ${i.hidden} restorable, ${i.removed} gone for good`;
      case "restore":
        return "Restored";
    }
  }

  function summaryText(s: OperationSummary): string {
    const verb = VERBS[s.kind].toLowerCase();
    const parts: string[] = [];
    if (s.items.completed > 0) parts.push(`${s.items.completed} done`);
    if (s.items.skipped > 0) parts.push(`${s.items.skipped} skipped`);
    if (s.items.partial > 0) parts.push(`${s.items.partial} partly done`);
    if (s.items.failed > 0) parts.push(`${s.items.failed} failed`);
    if (s.items.cancelled > 0) parts.push(`${s.items.cancelled} not started`);
    const counts = parts.join(", ");
    switch (s.state) {
      case "completed":
        return `Finished: ${counts}.`;
      case "partial":
        return `The ${verb} finished with problems: ${counts}.`;
      case "cancelled":
        return `Cancelled: ${counts}.`;
      case "failed":
        return `Stopped after an error: ${counts}.`;
    }
  }

  const percent = $derived(
    phase.step === "running" || phase.step === "finished"
      ? phase.progress.total === 0
        ? 100
        : Math.floor((phase.progress.done / phase.progress.total) * 100)
      : 0,
  );
</script>

{#if phase.step === "planning"}
  <div class="operation" role="status" aria-label="File operation">Checking what this would change…</div>
{/if}

{#if planError !== null}
  <div class="operation failed" role="alert" aria-label="File operation">
    <p>{describeOperationError(planError)}</p>
    <div class="actions">
      {#if planError instanceof CloudCommandError && planError.kind === "credentials-missing"}
        <button type="button" onclick={onedit}>Edit account</button>
      {/if}
      <button type="button" onclick={dismiss}>Dismiss</button>
    </div>
  </div>
{/if}

{#if phase.step === "review"}
  {@const p = phase.plan}
  {@const usable = p.items.filter((i) => i.problem === null)}
  <Modal title={reviewTitle(p)} onclose={closeReview} locked={phase.starting} wide>
    <div class="review" data-plan-kind={p.kind}>
      {#if p.kind !== "rename" || usable.length !== 1}
        <p>
          {p.objects.toLocaleString()} object{p.objects === 1 ? "" : "s"} ({formatSize(p.bytes)}) in {usable.length} item{usable.length === 1
            ? ""
            : "s"}.
        </p>
      {:else if usable[0] !== undefined}
        <p>
          From <span class="key">{usable[0].source}</span> to <span class="key">{usable[0].destination}</span>
          {#if usable[0].folder}({usable[0].objects.toLocaleString()} objects){/if}.
        </p>
      {/if}
      {#if p.kind === "delete"}
        <p class="note" class:warning={p.versioning !== "enabled"}>{versioningNote(p)}</p>
        <p class="note">Only the objects listed now are deleted; nothing next to them (such as a folder whose name starts the same way).</p>
      {:else if p.kind === "move" || p.kind === "rename"}
        <p class="note">{moveNote(p)}</p>
        {#if p.versioning === "enabled"}
          <p class="note">On this versioned bucket the originals stay restorable from Deleted items.</p>
        {/if}
      {:else if p.kind === "copy"}
        <p class="note">Copies keep the content type and metadata (including sotyware's SHA-256 checksum); they get a new date.</p>
      {:else if p.kind === "restore"}
        <p class="note">Restoring removes the delete markers, so the newest kept version becomes current again.</p>
      {/if}
      <ul class="plan-items">
        {#each p.items.slice(0, 100) as item (item.index)}
          <li class:problem={item.problem !== null} class:conflict={item.conflict !== null}>
            <span class="item-icon" aria-hidden="true"><Icon name={item.folder ? "folder" : "file"} size={14} /></span>
            <strong class="key">{shownName(item.name)}</strong>
            {#if item.problem === null}
              <span class="detail">
                {item.folder ? `${item.objects.toLocaleString()} objects, ` : ""}{formatSize(item.bytes)}
              </span>
            {/if}
            {#if item.problem !== null}
              <span class="detail">Left out: {item.problem}.</span>
            {:else if item.conflict !== null}
              <span class="detail">
                Already exists at <span class="key">{item.conflict.existing}</span>{#if item.conflict.alternative !== null};
                  “Keep both” saves it as <span class="key">{item.conflict.alternative}</span>{:else}; no free name was found{/if}.
              </span>
            {/if}
          </li>
        {/each}
        {#if p.items.length > 100}
          <li>…and {p.items.length - 100} more.</li>
        {/if}
      </ul>
      {#if p.conflicts > 0}
        <fieldset class="conflicts">
          <legend>{p.conflicts} item{p.conflicts === 1 ? "" : "s"} already exist{p.conflicts === 1 ? "s" : ""} at the destination</legend>
          <label><input type="radio" name="policy" value="skip" bind:group={phase.policy} /> Skip them (nothing is replaced)</label>
          <label><input type="radio" name="policy" value="rename" bind:group={phase.policy} /> Keep both (save with a numbered name)</label>
        </fieldset>
      {/if}
      {#if phase.error !== null}
        <p role="alert" class="error">{describeOperationError(phase.error)}</p>
      {/if}
    </div>
    {#snippet footer()}
      {#if phase.step === "review"}
        {#if (phase.plan.kind === "copy" || phase.plan.kind === "move") && onrechoose}
          <button type="button" disabled={phase.starting} onclick={rechoose}>Choose another destination</button>
        {/if}
        <button type="button" disabled={phase.starting} onclick={closeReview}>Cancel</button>
        <button
          type="button"
          class="primary"
          class:danger={phase.plan.kind === "delete"}
          data-autofocus={phase.plan.kind !== "delete" ? true : undefined}
          disabled={phase.starting || phase.plan.items.every((i) => i.problem !== null) ||
            (phase.policy === "skip" && phase.plan.items.every((i) => i.problem !== null || i.conflict !== null))}
          onclick={() => void confirm()}>{phase.starting ? "Starting…" : VERBS[phase.plan.kind]}</button
        >
      {/if}
    {/snippet}
  </Modal>
{/if}

{#if phase.step === "running" || phase.step === "finished"}
  <div
    class="operation"
    class:failed={phase.summary !== null && phase.summary.state !== "completed"}
    role="region"
    aria-label="File operation"
    data-operation-state={phase.summary?.state ?? "running"}
  >
    <p class="title"><strong>{VERBS[phase.kind]}</strong></p>
    {#if phase.step === "running"}
      <progress max="100" value={percent} aria-label={`${VERBS[phase.kind]} progress`}>{percent}%</progress>
      <p role="status">
        {phase.cancel ? "Cancelling after the requests in flight…" : `${phase.progress.done.toLocaleString()} of ${phase.progress.total.toLocaleString()} objects`}
      </p>
    {/if}
    {#if phase.summary !== null}
      <p role="status" class="result">{summaryText(phase.summary)}</p>
      {#if phase.summary.kind === "delete" && (phase.summary.hidden > 0 || phase.summary.removed > 0)}
        <p class="note">
          {#if phase.summary.hidden > 0}{phase.summary.hidden.toLocaleString()} object{phase.summary.hidden === 1 ? "" : "s"} can be restored from Deleted items.{/if}
          {#if phase.summary.removed > 0}{phase.summary.removed.toLocaleString()} object{phase.summary.removed === 1 ? " is" : "s are"} gone for good.{/if}
        </p>
      {/if}
      {#if phase.summary.error !== null}
        <p role="alert">{describeOperationFailure(phase.summary.error)}</p>
      {/if}
    {/if}
    <ul class="op-items">
      {#each phase.items as item (item.index)}
        <li data-item-state={item.state}>
          <span class="key">{shownName(item.name)}</span>: {itemLine(item)}
          {#if item.failures.length > 0}
            <ul class="failures">
              {#each item.failures as f (f.key)}
                <li><span class="key">{f.key}</span>: {describeOperationFailure(f.error)}</li>
              {/each}
              {#if item.objectsFailed > item.failures.length}
                <li>…and {item.objectsFailed - item.failures.length} more.</li>
              {/if}
            </ul>
          {/if}
        </li>
      {/each}
    </ul>
    {#if phase.error !== null}
      <p role="alert">{describeOperationError(phase.error)}</p>
    {/if}
    <div class="actions">
      {#if phase.step === "running"}
        <button type="button" disabled={phase.cancel} onclick={() => void cancel()}>Cancel</button>
      {:else}
        {#if phase.summary?.retryable}
          <button type="button" onclick={() => void retry()}>Try again</button>
        {/if}
        <button type="button" onclick={dismiss}>Dismiss</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .operation {
    margin: var(--space-3) 0;
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
    font-size: var(--text-sm);
  }

  .operation.failed {
    border-color: var(--danger-border);
  }

  .operation p,
  .review p {
    margin: var(--space-1) 0;
    overflow-wrap: anywhere;
  }

  .operation progress {
    margin: var(--space-2) 0;
  }

  .key {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .note {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .note.warning {
    padding: var(--space-2) var(--space-3);
    color: var(--text);
    font-weight: var(--weight-medium);
    background: var(--warning-soft);
    border: 1px solid var(--warning-border);
    border-radius: var(--radius-md);
  }

  .plan-items,
  .op-items {
    margin: var(--space-3) 0;
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-6);
    max-height: 16rem;
    overflow: auto;
    font-size: var(--text-sm);
    background: var(--surface-2);
    border-radius: var(--radius-md);
  }

  .plan-items li,
  .op-items li {
    padding: 2px 0;
  }

  .plan-items li.problem {
    color: var(--text-muted);
  }

  .plan-items li.conflict .detail {
    color: var(--warning);
  }

  .detail {
    margin-left: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .failures {
    font-size: var(--text-xs);
    color: var(--danger);
  }

  fieldset.conflicts {
    margin: var(--space-3) 0;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  fieldset.conflicts label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-1) 0;
    font-size: var(--text-sm);
  }

  .operation .actions {
    margin-top: var(--space-2);
  }

  .item-icon {
    display: inline-flex;
    vertical-align: -2px;
    color: var(--text-subtle);
  }
</style>
