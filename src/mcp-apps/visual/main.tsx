/**
 * The MCP Apps view for `ui://bridge/visual`.
 *
 * Runs inside a sandboxed iframe with no network: the host hands it the tool
 * input over the MCP Apps bridge, and it draws. Fonts are inlined (only the
 * Latin subsets, as data URLs) because the frame's CSP allows nothing else.
 */

import "@/index.css";
import { App } from "@modelcontextprotocol/ext-apps";
import { createRoot } from "react-dom/client";
import geistLatin from "@fontsource-variable/geist/files/geist-latin-wght-normal.woff2";
import geistMonoLatin from "@fontsource-variable/geist-mono/files/geist-mono-latin-wght-normal.woff2";
import { parseVisualSpec } from "../spec";
import { VisualView } from "./VisualView";

const LATIN = "U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD";

for (const [family, source] of [["Geist Variable", geistLatin], ["Geist Mono Variable", geistMonoLatin]] as const) {
  const face = new FontFace(family, `url(${source}) format("woff2")`, { weight: "100 900", style: "normal", display: "swap", unicodeRange: LATIN });
  document.fonts.add(face);
  void face.load().catch(() => undefined);
}

// The card behind the frame is the surface; the document stays transparent.
document.documentElement.classList.add("bg-transparent");
document.body.classList.add("bg-transparent");

function applyTheme(theme: string | undefined) {
  if (theme !== "dark" && theme !== "light") return;
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.dataset.theme = theme;
}

const mount = document.createElement("div");
mount.id = "visual-root";
document.body.append(mount);
const root = createRoot(mount);

const app = new App({ name: "bridge-visual", version: "1.0.0" }, {}, { autoResize: true });

function draw(input: unknown) {
  const parsed = parseVisualSpec(input);
  if (!parsed.ok) {
    root.render(
      <p className="px-5 py-4 text-[12.5px] text-muted-foreground">This visual could not be drawn: {parsed.errors[0]?.path || "spec"} {parsed.errors[0]?.message}</p>,
    );
    return;
  }
  // Bridge's card already shows the title; any other MCP Apps host gets it here.
  const showTitle = app.getHostVersion()?.name !== "bridge";
  root.render(
    <VisualView
      spec={parsed.spec}
      showTitle={showTitle}
      actions={{
        ask: text => void app.sendMessage({ role: "user", content: [{ type: "text", text }] }),
        open: url => void app.openLink({ url }),
      }}
    />,
  );
}

app.ontoolinput = ({ arguments: input }) => draw(input);
app.onhostcontextchanged = context => applyTheme(context.theme);

void app.connect().then(() => applyTheme(app.getHostContext()?.theme));
