<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import Logo from "./Logo.svelte";

  // The dashboard's own title bar (ADR-040, docs/architecture/frontend.md):
  // drawn by App.svelte only where the window has no native decorations
  // (Windows). Its empty parts are Tauri's drag region: dragging moves the
  // window with the system's own move loop (Aero Snap included) and a double
  // click maximizes or restores it. Buttons and links are never part of it.
  // The three window controls call the window API, whose permissions Rust
  // grants only to an undecorated window (src-tauri/src/title_bar.rs).
  // Closing goes through the same close request as Alt+F4, which Rust turns
  // into hiding in tray mode.

  interface Props {
    /** The page shown, named in narrow windows (where the sidebar is hidden). */
    title: string;
    /** "tray": closing hides; "dashboard-only": closing quits; null: unknown. */
    mode: "tray" | "dashboard-only" | null;
    sidebarOpen: boolean;
    ontogglesidebar: () => void;
  }

  let { title, mode, sidebarOpen, ontogglesidebar }: Props = $props();

  let maximized = $state(false);
  let inactive = $state(false);
  let error = $state<string | null>(null);

  // Windows' caption glyphs (Segoe Fluent Icons on Windows 11, the same code
  // points in Segoe MDL2 Assets on Windows 10): crisp at every scaling.
  const GLYPH = { minimize: "", maximize: "", restore: "", close: "" };

  const closeTitle = $derived(
    mode === "tray" ? "Close (sotyware keeps running in the system tray)" : "Close",
  );

  // The maximized state decides the middle button's icon. It is read again
  // after each resize of the page (the window resizes when it is maximized
  // or restored); one query at a time, plus one more if a resize came in
  // meanwhile.
  let querying = false;
  let again = false;
  async function readMaximized(): Promise<void> {
    if (querying) {
      again = true;
      return;
    }
    querying = true;
    try {
      do {
        again = false;
        maximized = await getCurrentWindow().isMaximized();
      } while (again);
    } catch {
      // Unknown: keep the last value (the icon only).
    } finally {
      querying = false;
    }
  }

  async function control(action: "minimize" | "toggle" | "close"): Promise<void> {
    const w = getCurrentWindow();
    try {
      if (action === "minimize") {
        await w.minimize();
      } else if (action === "toggle") {
        await w.toggleMaximize();
        await readMaximized();
      } else {
        await w.close();
      }
      error = null;
    } catch (e) {
      const what = action === "minimize" ? "minimize" : action === "toggle" ? (maximized ? "restore" : "maximize") : "close";
      error = `Could not ${what} the window: ${String(e)}`;
    }
  }

  onMount(() => {
    void readMaximized();
    inactive = typeof document !== "undefined" && !document.hasFocus();
  });
</script>

<svelte:window
  onresize={() => void readMaximized()}
  onfocus={() => (inactive = false)}
  onblur={() => (inactive = true)}
/>

<!-- "deep": every part of the bar that is not a control starts a drag. -->
<div class="titlebar" class:inactive data-tauri-drag-region="deep" data-titlebar>
  <div class="brand-cell">
    <button
      type="button"
      class="nav-toggle"
      aria-label={sidebarOpen ? "Hide navigation" : "Show navigation"}
      aria-expanded={sidebarOpen}
      onclick={ontogglesidebar}><Icon name="sidebar" size={16} /></button
    >
    <span class="brand"><Logo size={18} /><span class="name">sotyware</span></span>
  </div>
  <div class="middle">
    <span class="page-title">{title}</span>
    {#if error !== null}
      <span class="error-text" role="alert">{error}</span>
    {/if}
  </div>
  <!-- Out of the tab order like the system's own caption buttons; the
       keyboard has Alt+Space, Win+Arrow and Alt+F4 for the same actions. -->
  <div class="controls" role="group" aria-label="Window">
    <button type="button" class="control" tabindex="-1" aria-label="Minimize" title="Minimize" onclick={() => void control("minimize")}
      ><span class="glyph" aria-hidden="true">{GLYPH.minimize}</span></button
    >
    <button
      type="button"
      class="control"
      tabindex="-1"
      aria-label={maximized ? "Restore" : "Maximize"}
      title={maximized ? "Restore Down" : "Maximize"}
      onclick={() => void control("toggle")}
      ><span class="glyph" aria-hidden="true">{maximized ? GLYPH.restore : GLYPH.maximize}</span></button
    >
    <button type="button" class="control close" tabindex="-1" aria-label="Close" title={closeTitle} onclick={() => void control("close")}
      ><span class="glyph" aria-hidden="true">{GLYPH.close}</span></button
    >
  </div>
</div>

<style>
  /* Above dialog backdrops (Modal.svelte), as a native title bar is: the
     window can be moved, minimized or closed while a dialog is open. */
  .titlebar {
    position: relative;
    z-index: calc(var(--z-dialog) + 1);
    grid-area: titlebar;
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr) auto;
    align-items: stretch;
    height: var(--titlebar-h);
    min-width: 0;
    background: var(--bg);
    color: var(--text);
    user-select: none;
    -webkit-user-select: none;
  }

  /* Over the sidebar: the sidebar's colour and its right edge, so the
     sidebar reads as one column from the top of the window. */
  .brand-cell {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-4);
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    min-width: 0;
  }

  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .name {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    letter-spacing: -0.005em;
  }

  .middle {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    padding: 0 var(--space-4);
  }

  /* The section's name only where the sidebar does not show it. */
  .page-title {
    display: none;
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .error-text {
    min-width: 0;
    font-size: var(--text-xs);
    color: var(--danger);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .controls {
    display: flex;
    align-items: stretch;
  }

  /* Windows 11's caption buttons: 46 px wide, the bar's full height, square,
     a light fill on hover, the system's red for Close. */
  .control {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: var(--titlebar-h);
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text);
    box-shadow: none;
    transition: background-color var(--dur-fast) var(--ease);
  }

  .control:hover {
    background: var(--titlebar-hover);
  }

  .control:active {
    background: var(--titlebar-pressed);
  }

  .control.close:hover {
    background: var(--titlebar-close);
    color: var(--titlebar-close-text);
  }

  .control.close:active {
    background: var(--titlebar-close-pressed);
    color: var(--titlebar-close-text);
  }

  .control:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }

  .glyph {
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 10px;
    line-height: 1;
  }

  /* Like the system's title bars: dimmed while another window is active. */
  .inactive .name,
  .inactive .page-title,
  .inactive .control:not(:hover) {
    color: var(--text-subtle);
  }

  .nav-toggle {
    display: none;
    width: 28px;
    height: 28px;
    min-height: 0;
    padding: 0;
    border: 0;
    background: transparent;
    box-shadow: none;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
  }

  .nav-toggle:hover {
    background: var(--titlebar-hover);
    color: var(--text);
  }

  /* Narrow windows: the sidebar is an overlay, so the bar takes over the
     narrow top bar's job (navigation toggle and the page's name). */
  @media (max-width: 760px) {
    .titlebar {
      grid-template-columns: auto minmax(0, 1fr) auto;
      border-bottom: 1px solid var(--border);
    }

    .brand-cell {
      padding: 0 var(--space-2) 0 var(--space-3);
      background: transparent;
      border-right: 0;
    }

    .nav-toggle {
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }

    .brand {
      display: none;
    }

    .middle {
      padding: 0 var(--space-2);
    }

    .page-title {
      display: inline;
    }
  }
</style>
