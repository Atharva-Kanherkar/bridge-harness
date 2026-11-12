import fs from "node:fs";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { configDefaults } from "vitest/config";

// Excalidraw's fonts, served and bundled under /excalidraw/fonts so drawings
// in Notes never fetch them from a CDN (see drawingAssets.ts). Xiaolai, the
// 12 MB CJK face, stays on Excalidraw's own fallback.
const excalidrawFonts = path.resolve(__dirname, "node_modules/@excalidraw/excalidraw/dist/prod/fonts");
const SKIPPED_FONTS = new Set(["Xiaolai"]);

function excalidrawAssets(): Plugin {
  return {
    name: "bridge-excalidraw-fonts",
    configureServer(server) {
      server.middlewares.use("/excalidraw/fonts", (req, res, next) => {
        const file = path.join(excalidrawFonts, decodeURIComponent((req.url ?? "").split("?")[0]));
        if (!file.startsWith(excalidrawFonts + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) return next();
        res.setHeader("Content-Type", "font/woff2");
        fs.createReadStream(file).pipe(res);
      });
    },
    generateBundle() {
      if (!fs.existsSync(excalidrawFonts)) return;
      for (const family of fs.readdirSync(excalidrawFonts)) {
        if (SKIPPED_FONTS.has(family)) continue;
        for (const file of fs.readdirSync(path.join(excalidrawFonts, family))) {
          this.emitFile({ type: "asset", fileName: `excalidraw/fonts/${family}/${file}`, source: fs.readFileSync(path.join(excalidrawFonts, family, file)) });
        }
      }
    },
  };
}

export default defineConfig(({ mode }) => ({
  plugins: [react(), tailwindcss(), excalidrawAssets()],
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
  // Keep Vitest's recursive dependency exclusions: packaging stages SDKs with
  // their own tests under resources/ and target/, not just root node_modules/.
  test: {
    // The sidecar and release scripts have dedicated runners.
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    // Excalidraw's package imports JSON without an import attribute, which
    // Node refuses; transforming it through Vite lets drawing tests load it.
    server: { deps: { inline: [/@excalidraw\//] } },
    exclude: [
      ...configDefaults.exclude,
      "src-tauri/target/**",
      "src-tauri/resources/**",
      ".worktrees/**",
      ".claude/worktrees/**",
      ".codex-worktrees/**",
    ],
  }
}));
