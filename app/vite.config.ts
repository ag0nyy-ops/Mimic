import { defineConfig } from "vite";

// tauri expects a fixed dev port and doesn't want the screen cleared
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/core/**", "**/target/**"] },
  },
  build: {
    target: "chrome105", // webview2 is evergreen chromium
    outDir: "dist",
    emptyOutDir: true,
  },
});
