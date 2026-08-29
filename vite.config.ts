import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig(({ mode }) => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src")
    }
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "safari13", minify: mode === "development" ? false : "esbuild", sourcemap: mode === "development" },
  // Bridge and its coding agents create task worktrees inside the repo
  // (.worktrees/ and .claude/worktrees/). They are checkouts of other
  // branches, so their tests belong to those branches, not to this run.
  test: { exclude: ["node_modules/**", "dist/**", ".worktrees/**", ".claude/worktrees/**"] }
}));
