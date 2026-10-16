# Safari clone sign-in: Test Contract

## Functional Behavior
- Safari is selectable in browser-copy settings, approval cards, and manual starts.
- Safari always uses fresh sign-in in the existing guarded Chromium clone. UI names the engine and explains that Safari cookies are not copied.
- Safari requests, including stale clients asking for import, never invoke the cookie importer; the agent remains paused until the person hands control back.
- Existing Chrome defaults and saved settings remain compatible. Brave import remains available.
- The same site scope, RAM profile, TTL, takeover, turn cleanup and leak guard apply.
- Native WebKit and the optional Full Disk Access importer are outside this first slice.

## Unit Tests
- Protocol deserializes old settings/approval payloads with Chrome defaults.
- Orchestrator handles Safari + import as fresh sign-in without calling importer.
- Settings and consent tests verify Safari selection, forced fresh sign-in and engine disclosure.
- Mock API matches native Safari behavior.

## Integration / Functional Tests
- Build and full test suite pass; generated wire artifacts match their sources.
- Approval dispatch carries selected browser to the orchestrator.

## Smoke Tests
- Fresh-sign-in synthetic clone starts, pauses the agent, supports handback and destroys cleanly.

## E2E Tests
- Live Safari-source clone uses the existing Chromium engine, whose guarded lifecycle has existing live coverage. No Safari cookie access is needed.

## Manual / cURL Tests
- Choose Safari under Settings > Browser copies. Allow a site request: card says Chromium and no cookie copy; finish sign-in in the Browser pane, then hand back. Destroy removes the temporary session.
