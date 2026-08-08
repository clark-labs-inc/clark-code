import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

// Tauri expects a fixed port and no auto-clearing of the console.
const host = process.env.TAURI_DEV_HOST;
const productionSourcemaps = process.env.VITE_BUILD_SOURCEMAP === "1";
const neutralProductEntry = fileURLToPath(
  new URL("./src/product/neutralEntry.ts", import.meta.url),
);
const productEntry = process.env.DESKTOP_PRODUCT_ENTRY || neutralProductEntry;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@product-entry": productEntry,
    },
  },
  // Prevent Vite from obscuring Rust errors.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // Don't watch the Rust side.
      ignored: ["**/src-tauri/**"],
    },
  },
  // Produce a relative-path build that the Tauri webview can load from disk.
  build: {
    target: "es2022",
    sourcemap: productionSourcemaps,
    chunkSizeWarningLimit: 800,
  },
});
