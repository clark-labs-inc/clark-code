import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed port and no auto-clearing of the console.
const host = process.env.TAURI_DEV_HOST;
const productionSourcemaps = process.env.VITE_BUILD_SOURCEMAP === "1";

export default defineConfig({
  plugins: [react(), tailwindcss()],
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
    rolldownOptions: {
      output: {
        codeSplitting: {
          minSize: 20_000,
          groups: [
            {
              name: "react",
              test: /node_modules[\\/](react|react-dom|scheduler)[\\/]/,
              priority: 3,
            },
            {
              name: "markdown",
              test: /node_modules[\\/](react-markdown|(?:remark|rehype|micromark|mdast|hast)[^\\/]*|unified|katex)[\\/]/,
              priority: 2,
              maxSize: 350_000,
            },
            {
              name: "motion",
              test: /node_modules[\\/](motion|framer-motion)[\\/]/,
              priority: 2,
            },
          ],
        },
      },
    },
  },
});
