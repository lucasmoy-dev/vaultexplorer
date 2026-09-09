import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // The QR decoder runs in a worker. Classic rather than module workers
  // because this window is WebKitGTK on Linux, and a scanner that silently
  // fails to start there is the whole feature gone.
  worker: { format: "iife" },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
