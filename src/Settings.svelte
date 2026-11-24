<script lang="ts">
  import Features from "./Features.svelte";
  import Feedback from "./Feedback.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import type { SettingsTab } from "./navigation";
  import NotesOverlaySettings from "./NotesOverlaySettings.svelte";
  import type { OverlayModel } from "./overlayModel.svelte";
  import { applyTheme, applyWindowTheme, loadTheme, saveTheme, THEME_CHOICES, type ThemeChoice } from "./theme";
  import Updates from "./Updates.svelte";
  import type { UpdatesModel } from "./updatesModel.svelte";

  // Settings: appearance (this device only), the built-in features, the
  // gaming notes overlay (ADR-052), and About & updates (with the feedback
  // buttons). Tabs down the left like a desktop settings window,
  // across the top in a narrow window.

  type OperatingMode = "tray" | "dashboard-only";

  interface Props {
    tab: SettingsTab;
    ontab: (tab: SettingsTab) => void;
    updates: UpdatesModel;
    mode: OperatingMode | null;
    overlay: OverlayModel;
  }

  let { tab, ontab, updates, mode, overlay }: Props = $props();

  const TABS: { id: SettingsTab; title: string; icon: IconName }[] = [
    { id: "general", title: "General", icon: "settings" },
    { id: "features", title: "Features", icon: "feature" },
    { id: "overlay", title: "Notes overlay", icon: "note" },
    { id: "about", title: "About & updates", icon: "info" },
  ];

  const THEME_ICONS: Record<ThemeChoice, IconName> = { system: "monitor", light: "sun", dark: "moon" };

  const mac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);

  let theme = $state<ThemeChoice>(loadTheme());
  let themeSaveFailed = $state(false);

  function chooseTheme(choice: ThemeChoice): void {
    theme = choice;
    applyTheme(choice);
    void applyWindowTheme(choice);
    themeSaveFailed = !saveTheme(choice);
  }

  function onTabKey(event: KeyboardEvent): void {
    const i = TABS.findIndex((t) => t.id === tab);
    const step = event.key === "ArrowDown" || event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1 : 0;
    if (step === 0 && event.key !== "Home" && event.key !== "End") {
      return;
    }
    event.preventDefault();
    const next =
      event.key === "Home" ? TABS[0] : event.key === "End" ? TABS.at(-1) : TABS[(i + step + TABS.length) % TABS.length];
    if (next) {
      ontab(next.id);
      queueMicrotask(() => document.getElementById(`settings-tab-${next.id}`)?.focus());
    }
  }
</script>

<div class="page settings">
  <header class="page-header">
    <div class="titles">
      <h1>Settings</h1>
    </div>
  </header>

  <div class="layout">
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div class="tabs" role="tablist" aria-label="Settings" aria-orientation="vertical" tabindex="-1" onkeydown={onTabKey}>
      {#each TABS as t (t.id)}
        <button
          type="button"
          role="tab"
          id="settings-tab-{t.id}"
          class="tab"
          aria-selected={tab === t.id}
          aria-controls="settings-panel"
          tabindex={tab === t.id ? 0 : -1}
          onclick={() => ontab(t.id)}><Icon name={t.icon} />{t.title}</button
        >
      {/each}
    </div>

    <div class="panel" id="settings-panel" role="tabpanel" aria-labelledby="settings-tab-{tab}">
      {#if tab === "general"}
        <section aria-labelledby="appearance-heading">
          <h2 id="appearance-heading">Appearance</h2>
          <p class="muted small-text">Applies to this computer only.</p>
          <div class="themes" role="radiogroup" aria-label="Theme">
            {#each THEME_CHOICES as c (c.id)}
              <label class="theme" class:chosen={theme === c.id}>
                <input type="radio" name="theme" value={c.id} checked={theme === c.id} onchange={() => chooseTheme(c.id)} />
                <span class="swatch {c.id}" aria-hidden="true"><span></span></span>
                <span class="theme-name"><Icon name={THEME_ICONS[c.id]} size={14} />{c.label}</span>
              </label>
            {/each}
          </div>
          {#if themeSaveFailed}
            <p class="callout warning" role="status">
              The theme could not be saved on this computer; it applies until sotyware quits.
            </p>
          {/if}
        </section>

        <section aria-labelledby="window-heading" class="group">
          <h2 id="window-heading">Window</h2>
          <dl class="meta-list">
            <dt>Closing the window</dt>
            <dd>
              {#if mode === "tray"}
                Hides it; sotyware keeps running in the {mac ? "menu bar" : "system tray"}. Quit from the tray menu{mac ? " or with ⌘Q" : ""}.
              {:else if mode === "dashboard-only"}
                Quits sotyware: no system tray is available on this desktop.
              {:else}
                Unknown.
              {/if}
            </dd>
            <dt>Keyboard</dt>
            <dd>
              <kbd>{mac ? "⌘" : "Ctrl"}</kbd> + <kbd>1</kbd>–<kbd>4</kbd> switch sections, <kbd>{mac ? "⌘" : "Ctrl"}</kbd> +
              <kbd>,</kbd> opens Settings. In a bucket: <kbd>{mac ? "⌘" : "Ctrl"}</kbd> + <kbd>F</kbd> searches,
              <kbd>F2</kbd> renames, <kbd>{mac ? "⌘⌫" : "Delete"}</kbd> deletes after review.
            </dd>
          </dl>
        </section>
      {:else if tab === "features"}
        <Features />
      {:else if tab === "overlay"}
        <NotesOverlaySettings model={overlay} />
      {:else}
        <Updates model={updates} />
        <Feedback />
      {/if}
    </div>
  </div>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 12rem minmax(0, 1fr);
    gap: var(--space-8);
    align-items: start;
  }

  .tabs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    position: sticky;
    top: 0;
  }

  .tab {
    justify-content: flex-start;
    gap: var(--space-3);
    padding: 0 var(--space-3);
    background: transparent;
    border-color: transparent;
    color: var(--text-muted);
  }

  .tab:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }

  .tab[aria-selected="true"] {
    background: var(--surface-3);
    color: var(--text);
  }

  .panel {
    min-width: 0;
  }

  .panel h2 {
    margin-bottom: var(--space-1);
  }

  .group {
    margin-top: var(--space-8);
  }

  .group h2 {
    margin-bottom: var(--space-3);
  }

  .themes {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .theme {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease);
  }

  .theme:hover {
    border-color: var(--border-strong);
  }

  .theme.chosen {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .theme input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
  }

  .theme:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }

  .swatch {
    display: block;
    width: 7.5rem;
    height: 4.5rem;
    border-radius: var(--radius-md);
    border: 1px solid rgb(0 0 0 / 10%);
    overflow: hidden;
    position: relative;
  }

  .swatch span {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 32%;
  }

  .swatch.light {
    background: #faf9f7;
  }

  .swatch.light span {
    background: #f2f1ee;
    border-right: 1px solid rgb(0 0 0 / 8%);
  }

  .swatch.dark {
    background: #1f1e1c;
  }

  .swatch.dark span {
    background: #191817;
    border-right: 1px solid rgb(255 255 255 / 8%);
  }

  .swatch.system {
    background: linear-gradient(135deg, #faf9f7 50%, #1f1e1c 50%);
  }

  .swatch.system span {
    display: none;
  }

  .theme-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    padding: 0 var(--space-1);
  }

  @media (max-width: 760px) {
    .layout {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--space-4);
    }

    .tabs {
      position: static;
      flex-direction: row;
      overflow-x: auto;
      border-bottom: 1px solid var(--border);
      padding-bottom: var(--space-2);
    }
  }
</style>
