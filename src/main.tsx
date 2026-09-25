import ReactDOM from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/geist-mono";
import "@xterm/xterm/css/xterm.css";
import "katex/dist/katex.min.css";
import "./index.css";
import { App } from "./App";
import { dismissBootSplash } from "./bootSplash";
import { installExternalLinkHandler } from "./externalLinks";

installExternalLinkHandler();

if ("__TAURI_INTERNALS__" in window) document.documentElement.dataset.tauri = "";

const params = new URLSearchParams(window.location.search);
const preview = params.get("preview");
const root = ReactDOM.createRoot(document.getElementById("root")!);
if (preview === "right-rail") {
  void import("./previews/RightRailPreview").then(({ RightRailPreview }) => {
    root.render(<RightRailPreview />);
    dismissBootSplash();
  });
} else if (params.get("window") === "meter") {
  // The menu-bar meter runs in its own borderless window off the same bundle.
  // It must not mount App: that would boot a second copy of the whole desktop
  // shell — its polling, its subscriptions, its state — behind a 360pt panel.
  document.documentElement.dataset.surface = "meter";
  void import("./components/meter/MeterPanel").then(({ MeterPanel }) => {
    root.render(<MeterPanel />);
    dismissBootSplash();
  });
} else {
  root.render(<App />);
  dismissBootSplash();
}
