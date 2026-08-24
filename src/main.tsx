import { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "@xterm/xterm/css/xterm.css";
import "katex/dist/katex.min.css";
import "./index.css";
import { App } from "./App";
import { DockPreview } from "./components/DockPreview";
import { installExternalLinkHandler } from "./externalLinks";

installExternalLinkHandler();

// The right-hand dock mockup is a design surface — no runtime, no data — so it
// stays out of the app's own routing. In the browser it answers to #dock; in the
// desktop window, which has no address bar, ⌃⌥D swaps it in and back out.
function Root() {
  const [preview, setPreview] = useState(() => window.location.hash === "#dock");

  useEffect(() => {
    // Option rewrites `key` on macOS (⌥D types ∂), so the physical `code` is the
    // reliable half of this test — but it is empty under synthetic input.
    const onKeyDown = (event: KeyboardEvent) => {
      const isD = event.code === "KeyD" || event.key === "d" || event.key === "D" || event.key === "∂";
      if (event.ctrlKey && event.altKey && !event.metaKey && isD) {
        event.preventDefault();
        setPreview(value => !value);
      }
    };
    const onHashChange = () => setPreview(window.location.hash === "#dock");
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("hashchange", onHashChange);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("hashchange", onHashChange);
    };
  }, []);

  return preview ? <DockPreview /> : <App />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<Root />);
