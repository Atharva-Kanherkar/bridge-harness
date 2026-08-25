import ReactDOM from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "@xterm/xterm/css/xterm.css";
import "katex/dist/katex.min.css";
import "./index.css";
import { App } from "./App";
import { installExternalLinkHandler } from "./externalLinks";

installExternalLinkHandler();

if ("__TAURI_INTERNALS__" in window) document.documentElement.dataset.tauri = "";

const preview = new URLSearchParams(window.location.search).get("preview");
const root = ReactDOM.createRoot(document.getElementById("root")!);
if (preview === "right-rail") {
  void import("./previews/RightRailPreview").then(({ RightRailPreview }) => {
    root.render(<RightRailPreview />);
  });
} else {
  root.render(<App />);
}
