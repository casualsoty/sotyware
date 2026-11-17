<script lang="ts">
  import Icon from "./Icon.svelte";
  import { openFeedbackPage, type FeedbackKind } from "./feedback";

  // Settings › About & updates: two buttons that open a GitHub issue form in
  // the browser. Nothing is sent from here; the user writes and submits it.

  let error = $state<string | null>(null);

  async function open(kind: FeedbackKind): Promise<void> {
    error = null;
    try {
      await openFeedbackPage(kind);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
</script>

<section aria-labelledby="feedback-heading" class="feedback">
  <h2 id="feedback-heading">Feedback</h2>
  <p class="muted small-text">
    Opens a form on GitHub in your browser. A bug report is filled in with sotyware's version and your operating system;
    nothing is sent until you submit it. Issues are public: leave out keys, passwords and personal details.
  </p>
  <div class="actions">
    <button type="button" onclick={() => void open("feedback")}><Icon name="external" size={14} />Send feedback</button>
    <button type="button" onclick={() => void open("bug")}><Icon name="external" size={14} />Report a bug</button>
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</section>

<style>
  .feedback {
    margin-top: var(--space-8);
  }

  h2 {
    margin-bottom: var(--space-1);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
</style>
