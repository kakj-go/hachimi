import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid({ dev: false, hot: false })],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // The Solid plugin otherwise auto-injects jest-dom when it is hoisted by
    // another workspace package. Workbench tests intentionally use Vitest's
    // built-in assertions and keep this package's test surface self-contained.
    setupFiles: ["./src/test/jest-dom-disabled.ts"],
  },
});
