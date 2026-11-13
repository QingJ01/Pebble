/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { configDefaults } from "vitest/config";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  root: __dirname,
  clearScreen: false,
  worker: { format: "es" },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: [
        "**/.git/**",
        "**/.worktrees/**",
        "**/dist/**",
        "**/node_modules/**",
        "**/src-tauri/**",
        "**/target/**",
      ],
    },
  },
  test: {
    environment: "jsdom",
    exclude: [...configDefaults.exclude, "**/.worktrees/**"],
    globals: true,
  },
});
