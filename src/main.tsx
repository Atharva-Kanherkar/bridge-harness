import ReactDOM from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "@xterm/xterm/css/xterm.css";
import "katex/dist/katex.min.css";
import "./index.css";
import { App } from "./App";
import { installExternalLinkHandler } from "./externalLinks";

installExternalLinkHandler();

ReactDOM.createRoot(document.getElementById("root")!).render(<App />);
