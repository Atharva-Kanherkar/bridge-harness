# fix-composer-discovery-and-suggestions: Test Contract

## Functional Behavior
- Ordinary prose, including “The skill suggestion does not work”, never opens a skill recommendation menu or calls task-based skill recommendations.
- A slash token at the cursor opens explicit discovery. At the beginning it offers commands and skills; inside a sentence it offers installed skills and custom prompts only. URLs, paths, selected text, and unrelated trailing tokens do not open the menu.
- Accepting a slash match preserves the sentence before and after the token, returns focus and caret to the insertion, and never sends the message. Escape dismisses discovery; a new slash token reopens it.
- Inline installed skills and custom prompts expand on send for the current provider. Embedded builtins stay literal; leading builtins keep existing semantics.
- Empty composer copy explains / for skills/commands, $ for a provider chat, @ for files, and # for specialist agents where supported.
- Enabled AI completions decode actual Claude, Codex, and OpenCode frames, ignore user/reasoning/tool output, replace streamed text with final snapshots without duplication, and surface provider errors.
- Typing or changing chats rejects stale results; unrelated session updates do not restart completion. Pending obsolete drafts are skipped before reaching the backend.
- A capped or timed-out suggestion interrupts and recycles the hidden session so its late output cannot become the next draft’s suggestion.
- Ghost text appears only at the end, Tab accepts it, Shift+Tab retains normal backwards navigation, and command discovery takes precedence.
- AI completion remains opt-in. Welcome drafts can receive completion before a chat is created. Failures are visible and recover on retry/settings changes.

## Unit Tests
- Slash token detection/insertion: leading, embedded, cursor in middle, suffix preservation, URLs/paths, selection and token boundaries.
- Scheduler: debounce, empty/disabled, stale success/error, single flight, latest pending draft, cancellation and failure recovery.
- Composer: caret reporting, Tab/Shift+Tab, ghost visibility and accessible command hints.
- Rust completion buffer: each provider’s real frame format, incremental/final deduplication, reasoning/user exclusion, failures and limits.
- Rust slash expansion: explicit embedded known skills, multiple skills, unknown tokens, paths/URLs/code and embedded builtins.

## Integration / Functional Tests
- Real App against mocked bridgeApi: no automatic skill popup; slash discovery mid-sentence; preserve draft on mouse and keyboard acceptance; slow completion survives a session snapshot update.
- Welcome and aside composer regression checks for completion and slash discovery where offered.
- Fake adapter runtime verifies capped/interrupted warm-session recycling without real model billing.

## Smoke Tests
- bun run build and bun run test must pass before PR creation.
- Approved local browser session verifies command hints, ordinary prose, leading slash, embedded slash, cursor insertion, dismissal, and completion controls.

## E2E Tests
- Browser checks use the local development backend; provider decoding is covered with deterministic actual-wire fixtures and an isolated live provider completion if available.

## Manual / cURL Tests
- In an existing chat type ordinary prose: no skills popup.
- Type “Please use /” and choose an installed skill: preceding text survives. Move the cursor before existing words, insert / and select: suffix survives.
- Start a draft with /: existing local commands remain available. Type a URL or filesystem path: no popup.
- Enable Settings > Composer, choose an available model, type an unfinished sentence, wait and accept with Tab. Shift+Tab moves focus normally. A failed model reports unavailable without blocking sending.
- Confirm the empty composer advertises supported prefixes. Confirm an unsent welcome draft receives opt-in AI completion.
