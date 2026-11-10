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
  },
  // Frontend unit tests (`npm run test:frontend`): a simulated DOM, no browser
  // and no display. See docs/development/testing.md.
  test: {
    environment: "happy-dom",
    include: ["src/**/*.test.ts"],
  },
  // Under Vitest, Node would otherwise resolve Svelte's server build, which
  // cannot `mount()`. Only applied in test mode; `vite build` is unaffected.
  ...(process.env["VITEST"] ? { resolve: { conditions: ["browser"] } } : {}),
});
