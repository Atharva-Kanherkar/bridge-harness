# feat-unify-browser-pane

One Browser pane in the dock. It is the person's own browser by default; when the agent has a throwaway clone (requested, starting, acting, waiting, taken over) the same pane shows it, and returns to the person's pages when the clone is gone. The separate Clone pane is removed.

## 1. Dock
- `DOCK_PANES` no longer lists `clone`; every other chord keeps its place except those after it (Context moves up by one).
- A saved dock state whose pane is `clone` restores to `browser`.
- The Browser tab carries the attention dot when a clone needs the person (waiting for you, pending request or approval).

## 2. BrowserPane
- No clone: the address bar and tabs show, no clone viewport.
- Clone engaged: the clone viewport shows and the browser tabs are hidden but stay mounted (their tabs survive).
- Clone gone: back to the browser tabs.
- The toolbar offers "Open a throwaway copy of a site"; the start form has "Back to browser".
- Clone supervision reaches the host while the pane is hidden.

## 3. Agent pointer
- The runtime records where the agent's last click, hover, drag, or scroll landed as viewport fractions, and reports it in `clone_state` as `agentPointer` with its age.
- The live view draws a cursor with the agent's name at that spot, fades after six quiet seconds, and is hidden while the person holds the clone.
- Taking over or destroying the clone clears the pointer.
