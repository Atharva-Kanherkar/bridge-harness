import { SimpleBrowser, type SimpleBrowserProps } from "./SimpleBrowser";

// The dock's one browser: your own pages, not signed in to anything Bridge
// knows about. The throwaway signed-in copy an agent could ask for is switched
// off for now, so `CloneSurface` is not mounted here. It and the core's clone
// code are kept; `OFFER_BROWSER_CLONES` in `live_turn.rs` is the agent-side
// half of the same switch.

export function BrowserPane(browser: SimpleBrowserProps) {
  return <div className="relative h-full min-w-0">
    <SimpleBrowser {...browser} />
  </div>;
}
