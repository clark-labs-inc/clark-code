import { defineConfig } from "vitest/config";

const productTestRoot = process.env.DESKTOP_PRODUCT_TEST_ROOT;

export default defineConfig({
  resolve: {
    dedupe: ["react", "react-dom"],
  },
  test: {
    environment: "node",
    include: [
      "src/**/*.{test,spec}.{ts,tsx}",
      ...(productTestRoot ? [`${productTestRoot}/**/*.{test,spec}.{ts,tsx}`] : []),
    ],
    setupFiles: ["./src/test/setup.ts"],
  },
});
