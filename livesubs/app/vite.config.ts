import { defineConfig } from "vite";
import { resolve } from "path";

// Two pages, one bundle: the settings window and the subtitle overlay are
// separate windows of the same app, and the overlay deliberately shares
// nothing heavy with the settings UI -- it has to stay light enough to sit
// on top of a video call all day.
export default defineConfig({
  clearScreen: false,
  server: { port: 5183, strictPort: true },
  build: {
    target: "esnext",
    rollupOptions: {
      input: {
        settings: resolve(__dirname, "index.html"),
        overlay: resolve(__dirname, "overlay.html"),
      },
    },
  },
});
