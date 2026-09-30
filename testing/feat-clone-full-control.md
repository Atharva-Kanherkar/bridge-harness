# feat/clone-full-control

A clone the person approves is a real browser for the agent: it sees it, drives it, and opens signed in as the person by default.

## Contract

1. **Vision on by default.** `screenshot` saves a PNG under the tool directory and returns its path, width, height, url, and title. Its pixels are CSS pixels, so the x/y read off it are the ones click takes. The person can turn it off in the approval card or in Settings. When it is off, `screenshot` returns 403 and every other kind still works. Screenshot files are deleted with the clone.
2. **Full control.** `read_page` returns one line per visible element: ref, role, name, value/href/state, and centre point, with password values hidden. Actions: click / double_click / triple_click / right_click / hover (ref or x,y); drag; type; key (named keys and chords with commands for select-all/copy/paste/undo); form_input (input, select, checkbox, contenteditable); scroll (delta or ref); navigate (url, back, forward, reload; waits for load); focus; evaluate (page JavaScript); wait. Aliases from other browser tools resolve (`inspect`, `left_click`, `javascript`, ...). Unknown kinds are refused.
3. **Consent still binds.** Every action kind needs the approval; takeover pauses all agent access; navigate is limited to the approved domain and approved dependencies; text the person types in bursts of 4+ characters is scrubbed from agent results.
4. **Import is the default sign-in.** New settings default to `import`, 30 min, vision on. Blank ("sign in inside") is the opt-in.
5. **UI.** The dock pane has an address bar (status dot, domain, state, time left), a framed viewport, and a footer showing what the agent can see. On takeover the viewport takes the keyboard directly: characters go as one burst per pause, and named keys, clicks, wheel, and paste are forwarded. The approval card has the sign-in toggle, the vision switch, and the lifetime select.

## Evidence

- `clone_orchestrator::tests::live::the_agent_sees_and_fully_drives_an_approved_clone` (real Chrome, `BRIDGE_CLONE_LIVE=1`): the PNG's width and height equal the CSS viewport; click-by-ref, type, form_input on a select and a checkbox, Enter, and click Pay all land (verified via evaluate); vision off returns 403 for screenshot and 200 for read_page; the screenshot is gone after destroy.
- `clone_browser_tool` unit tests: kind aliases, gating of every action, vision flag lifecycle, URL validation, key/chord mapping.
- `api::clone_settings_round_trip_through_the_native_store`: import + vision are the defaults.
- `CloneSurface`, `CloneRequestInbox`, `ClonesPage` component tests: keyboard burst forwarding, vision indicator, vision switch reaching `resolveCloneRequest`, settings defaults and persistence.
