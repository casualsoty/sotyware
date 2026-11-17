<script lang="ts">
  import Icon from "./Icon.svelte";
  import { openGithubPage, type GithubPage } from "./githubPage";

  // Settings › About & updates: two buttons that open a GitHub issue form in
  // the browser, and the privacy notice. Nothing is sent from here; the user
  // writes and submits the issue.

  let error = $state<string | null>(null);

  async function open(page: GithubPage): Promise<void> {
    error = null;
    try {
      await openGithubPage(page);
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
  <p class="muted small-text privacy">
    sotyware collects nothing without an account.
    <button type="button" class="link" onclick={() => void open("privacy-notice")}>Privacy notice</button>
  </p>
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

  .privacy {
    margin-top: var(--space-4);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
</style>
