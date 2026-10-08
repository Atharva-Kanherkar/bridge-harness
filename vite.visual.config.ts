import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const INDEX_CSS = path.resolve(__dirname, "src/index.css");

/**
 * The view uses the app's single stylesheet, but Tailwind should only
 * generate the utilities the view's own sources use. Rewrites the source
 * directives of `src/index.css` for this build only; the file on disk and
 * the app build are untouched.
 */
function viewSources(): Plugin {
  return {
    name: "bridge-visual-sources",
    enforce: "pre",
    transform(code, id) {
      if (id.split("?")[0] !== INDEX_CSS) return null;
      return code
        .replace('@import "tailwindcss";', '@import "tailwindcss" source(none);\n@source "./mcp-apps";\n@source "./components/DiagramFigure.tsx";\n@source "./lib";')
        .replace(/^@source\s+"\.\.\/[^"]*";\s*$/gm, "");
    },
  };
}

/**
 * Builds the MCP Apps view (`ui://bridge/visual`) as one IIFE plus one
 * stylesheet, which `scripts/build-visual.mjs` inlines into a single HTML
 * document. Library mode inlines every asset, so the fonts travel as data URLs.
 */
export default defineConfig({
  plugins: [viewSources(), react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  logLevel: "warn",
  build: {
    target: "safari13",
    outDir: "node_modules/.cache/bridge-visual",
    emptyOutDir: true,
    cssCodeSplit: false,
    lib: {
      entry: path.resolve(__dirname, "src/mcp-apps/visual/main.tsx"),
      formats: ["iife"],
      name: "BridgeVisual",
      fileName: () => "visual.js",
      cssFileName: "visual",
    },
  },
});
