import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed dev server port (see src-tauri/tauri.conf.json `devUrl`).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Two pages: the dashboard and the gaming notes overlay's own window
    // (ADR-052), which loads only what it needs. Paths are relative to the
    // project root.
    rolldownOptions: {
      input: {
        main: "index.html",
        overlay: "overlay.html",
      },
    },
  },
  // Frontend unit tests (`npm run test:frontend`): a simulated DOM, no browser
  // and no display. See docs/development/testing.md.
  test: {
    environment: "happy-dom",
    include: ["src/**/*.test.ts"],
    // en-US as every test's default locale, the simulated webview's language,
    // whatever the machine's (src/testLocale.ts).
    setupFiles: ["src/testLocale.ts"],
  },
  // Under Vitest, Node would otherwise resolve Svelte's server build, which
  // cannot `mount()`. Only applied in test mode; `vite build` is unaffected.
  ...(process.env["VITEST"] ? { resolve: { conditions: ["browser"] } } : {}),
});
