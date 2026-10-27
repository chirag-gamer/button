import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig(() => ({
  // One HTML entry per window (spec 4); more arrive with their phases.
  build: {
    rollupOptions: {
      input: {
        dropdown: "dropdown.html",
        backdrop: "backdrop.html",
        lock: "lock.html",
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
