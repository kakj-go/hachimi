import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

const root = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  build: {
    target: "es2023",
    rollupOptions: {
      input: {
        pet: fileURLToPath(new URL("pet.html", import.meta.url)),
        workbench: fileURLToPath(new URL("workbench.html", import.meta.url)),
        startup: fileURLToPath(new URL("startup.html", import.meta.url)),
      },
    },
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  root,
});
