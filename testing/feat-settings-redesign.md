# feat/settings-redesign: test contract

Locked before implementation. Three commit groups, in order: fixes, settings
redesign, model picker. Styling stays Tailwind v4 utilities on existing
Graphite & Paper tokens; no new CSS files, no inline style.

## Phase 1: broken settings

### 1. Input while running has one source

- The General page has no "Active-turn behavior" row and
  `ActiveTurnInputSetting` is deleted.
- `activeTurnSettings.ts` resolves the value as: the chat's own snapshot when
  its configuration revision is above 0, otherwise the application chat
  default `activeTurnInput`, otherwise `steer`.
- `App.tsx` and `AsideChat.tsx` both call the same resolver
  (`useActiveTurnInput(sessionSettings)`), so a legacy chat and its side chat
  agree.
- Saving "Input while running" on New chats updates the resolver in the same
  window without a reload.
- Migration, once: a legacy `bridge.composer.activeTurnInput = "queue"` is
  written into the application default when that default is unset; the
  legacy key is removed after a successful check. A set default is never
  overwritten. A failed save keeps the key so the next launch retries.

Tests (each fails on main):
- `activeTurnSettings.test.ts`: resolver precedence; migration writes queue
  when unset, leaves a set default alone, removes the key, keeps it on failure.
- `AsideChat.test.tsx`: the aside follows the application default.
- `App.drafts.test.tsx`: Queue mode comes from the application default.
- `AppearancePage`/General: no Active-turn row.

### 2. Per-row audit

Status: OK = control writes, value persists, a consumer reads it with a
visible effect. FIXED = broken on main, fixed here with a test. N/A = not
reachable on main by design.

| Page | Row | Storage | Consumer | Status |
| --- | --- | --- | --- | --- |
| General | Mode | localStorage `bridge.theme` + app prefs mirror | `src/theme.ts:175` applyTheme | OK |
| General | Window background | localStorage `bridge.skin` | `src/theme.ts:150` applySkin | OK |
| General | Window zoom | zoom store | `src/zoom.ts:279` setZoomLevel applies to webview | OK |
| General | Show thinking | localStorage `bridge.transcript.showThinking` | `src/components/AgentConversation.tsx:923` | OK |
| General | Open edit activity automatically | localStorage `bridge.transcript.autoExpandEditActivity` | `src/components/AgentConversation.tsx:924` | OK |
| General | Thinking control | localStorage `bridge.effortSelector` | `src/components/ChatModelControl.tsx:304` | OK |
| General | Show worker chats in Mission Control | localStorage `bridge.missionControl.showWorkerChats` | `src/components/MissionControl.tsx:382` | OK |
| General | Active-turn behavior | localStorage `bridge.composer.activeTurnInput` | `src/App.tsx:1696` ignores it when chat revision > 0; `src/components/AsideChat.tsx:84` reads only it | FIXED: row removed, value migrated to New chats default |
| General | Hub rows (Voice, Typing, Notifications, Menu bar, Updates) | none | none | FIXED: replaced by rail destinations |
| Appearance | whole page | same as General | same | FIXED: duplicate page, `appearance` aliases to `general` |
| Notifications | Toast/Banner/Sound per notice | localStorage `bridge.attention.settings` | `src/App.tsx:870` useAttentionSettings | OK |
| Notifications | Play | none (action) | `src/attentionSound.ts` | OK |
| Notifications | Quiet hours, From, Until | `bridge.attention.settings` | `src/attentionSettings.ts:88` isQuiet | OK |
| Voice | Set up / Remove local model, Retry status | backend voice model store | `voice/local_*` commands, `src/App.tsx` useVoiceDictation | OK |
| Typing & search | Inline suggestions Enabled, Suggestion model | `config/save_suggestion_settings` | `src/App.tsx:1787` + composer typeahead | OK |
| Typing & search | Search deeper, Search model | `chat_search/settings` | `src-tauri/bridge-core/src/chat_search/settings.rs:44` | OK |
| Menu bar | Display, icon layout, favorites, providers, usage, refresh | `menu_bar/save_settings` | `src-tauri/bridge-menu-bar/swift/MenuBar.swift` | OK |
| Menu bar | OpenCode workspace | `menu_bar/save_settings` on blur | same | OK (free text, kept on blur) |
| New chats | Apply defaults to (scope) | none (view filter) | page | OK |
| New chats | Agent, Model, Thinking, Access, Work location, Input while running | `config/save_chat_defaults` | `src-tauri/bridge-core/src/chat_defaults.rs:416` resolve | FIXED: only saved by a separate Save button; leaving the page lost the choice. Now saves on change with revision check |
| New chats | When a usage limit stops a chat: Action, Backup model, Backup effort | `limit_recovery` settings | `src-tauri/bridge-core/src/limit_recovery.rs:134` | FIXED: Backup effort had no Saved confirmation |
| Coding agents | Installed/Available rows, Install, Sign in | managed agents | `src/components/ManagedAgentsPanel.tsx` | OK |
| Coding agents | Hub rows (New chat defaults, Model preferences, Agents, Background tasks, Bridge instructions) | none | none | FIXED: replaced by rail destinations |
| Coding agent detail | Enabled, Default model, Thinking level | `config/save_harness` | `src/chatDefaults.ts:58` harnessLaunchOverrides, backend launch | OK |
| Coding agent detail | Accounts | harness `advanced.accounts` | `src/providerAccounts.ts` accountsOf, account picker | OK |
| Coding agent detail | Custom instructions, Executable path, Advanced JSON | `config/save_harness` via SaveBar | backend session launch | OK |
| Coding agent detail | OpenCode visible models | harness `advanced.visibleModels` | `src/components/SettingsScreen.tsx` modelOptions, picker | OK |
| Models | per profile: model, thinking, choice mode, backup, cost, speed, past results | `models/save_profiles` | `src-tauri/bridge-core/src/learning_router.rs:691`, `model_profiles.rs:380` | OK |
| Models | Available models Retry | catalog refresh | adapters | OK |
| Your agents | Enabled, What it does, Coding agent, Model, Thinking | `config/save_agent` on change | agent menu, `#` mention, chat target | OK |
| Your agents | Name, Description, Instructions | SaveBar | same | OK |
| Permissions | Let agents act without asking | `config/save_permission_policy` | `src-tauri/bridge-core/src/agent_config.rs:72` | OK |
| Permissions | Suggested instruction changes per role | `workerPromptProposalRoles` | `src-tauri/bridge-core/src/agent_config.rs:381` | OK |
| Permissions | Personal apps through OpenCode | app prefs `allowOpencodeAppCourier` | `src-tauri/bridge-core/src/apps/router.rs:59` | OK |
| Permissions | API keys hub row | none | none | FIXED: API keys is a rail destination |
| API keys | Add, Replace, Sites, Forget | Keychain via `secrets/*` | credential proxy | OK |
| Import | Import per tool | import job | `src/components/MoveToBridge.tsx` | OK |
| Archived chats | Search, Unarchive | `sessions/*archive*` | sidebar history | OK |
| Storage | Explorer, Trash, Worktree retention and cleanup | disk + `worktrees/*` | `src/components/settings/WorktreeStorage.tsx` | OK |
| Storage | Storage page duplicate (`storage` vs `data`) | same | same | FIXED: `storage` aliases to `data` |
| Background tasks | Workspace | none (view filter) | page | OK |
| Background tasks | Preferred agent, Try another agent, Automatic retry | `workers/save_settings` | `src-tauri/bridge-core/src/learning_router.rs:1198`, `live_turn.rs:8641`, `worker_settings.rs:25` | FIXED: only saved by a Save button |
| Background tasks | Tasks at once, per message, Stop waiting, Keep ready | `workers/save_settings` | `src-tauri/bridge-core/src/policy.rs`, `live_turn.rs:9365` | FIXED: Save button only; a cleared field sent 0, which the host refuses for Stop waiting. Now saves on valid change, out of range shows an error and does not send |
| Background tasks | Orchestrator notices | `delegation_notify` global | `src-tauri/bridge-core/src/delegation_notify.rs:101` | OK |
| Background tasks | Reviewer model and effort per harness | `reviewer` settings | GitHub pane Review launch | FIXED: selects only saved by a Save button; now save on change |
| Background tasks | Reviewer instructions | `reviewer` settings | same | OK (free text, SaveBar) |
| Bridge instructions | Sections, Reset, Restore, Export, Import | `prompts/*` | prompt compiler | OK |
| Daily briefing | Briefing on, harness, model, effort, connectors, frequency, focus | `work/write_settings` | `src/App.tsx:2225` refresh on focus; backend briefing | OK |
| Updates | Beta nightly builds, Check now | localStorage update channel | `src/App.tsx:1467` | OK |
| Footer | Reset all settings | `config/reset_all` + `models/reset_profiles` | config | FIXED: renamed to say what it resets |
| Browser copies | all | `clone/*` | n/a | N/A: page unmounted when clones were switched off; out of scope |
| Rail search | every hit | n/a | `SettingsScreen` | FIXED: hits only switched page; index was hand-kept and pointed at removed rows ("Menu Bar icon", `appearance`) |

Backend breaks found: none that a frontend change cannot fix.

### 3. Search reveals the row

- One registry (`settings/registry.ts`) lists every page (id, label,
  description, group) and every searchable row (id, page, label, description,
  `fold` when inside an Advanced fold). The rail and the search both read it;
  `STATIC_SETTINGS_ROWS` is gone.
- Rendered rows carry `data-settings-row="<id>"` through the kit (`anchor`).
- Clicking a hit opens the page, opens every enclosing `<details>`, scrolls
  the row into view and marks it `data-search-highlight` for about 1.6 s.
- Harness and agent hits open that harness's or agent's detail page.

Tests:
- `registry.test.tsx`: every registry row id renders on its page (mounted on
  mock data), so the index cannot go stale.
- `SettingsScreen.search.test.tsx`: a hit inside General's Advanced fold opens
  the fold, calls scrollIntoView, sets then clears the highlight; a harness
  hit opens the harness detail; an agent hit opens the agent detail.

### 4, 5, 6. Pages, aliases, contextual mode, reset label

- `canonicalSection`: `appearance -> general`, `storage -> data`,
  `harnesses -> codingAgents`, `history -> import`. Old ids still open.
- Page titles equal nav labels (test renders each page and compares its h2).
- `contextual` is removed; archives, saved-setups and briefing-settings views
  render the rail with the matching item current.
- The reset action reads "Reset agents, models and instructions".

## Phase 2: settings redesign

- Rail: left, `w-56`, full height, own scroll, no "Settings" heading. Groups:
  Basics (General, Notifications, Voice, Typing & search, Menu bar); Agents &
  models (New chats, Coding agents, Models, Your agents, Permissions, API
  keys); Data (Import, Archived chats, Storage); Advanced (Background tasks,
  Bridge instructions, Daily briefing), collapsed by default, open state
  remembered in localStorage. Footer: Updates (dot when an update is
  available) and the reset action.
- Below md the rail is a Select listing every page, grouped.
- Content column left aligned beside the rail, `max-w-176` (44rem), generous
  left padding.
- General: Mode, Window background, Zoom, Conversation, one Advanced fold
  (Thinking control, Mission Control).
- Kit: `SettingsPage` (title, description, groups), `SettingsFold` (the one
  Advanced fold), `Segmented` (2-3 options), `NumberField` (saves on valid
  change). No hub rows and no "Back to" links anywhere.
- Save rule: switches, selects, segmented and number fields save on change
  with the Saved flash; only free text uses SaveBar.

Tests: rail lists every page in one click; Advanced group remembers its
state; mobile Select lists every page; no "Back to" text; Workers and New
chats save on change with the revision passed; Saved flash shows.

## Phase 3: model picker

- One column, `w-95` (380px). Header: search and refresh. Under it a
  horizontal harness strip (favorites star, then one mark per harness); the lit
  item follows list scroll, click jumps, ArrowLeft/Right move within it.
- Sticky group header per harness or account: mark, label, right-aligned
  thinking and unavailable badges that do not truncate into each other.
- Rows `h-8`, fixed 16px leading gutter with a check on the selected row, no
  per-row mark. Selected: `bg-selection text-selection-foreground
  font-medium`. Hover: `bg-accent/50`. Right side: preview badge, star on
  hover or when starred.
- One full-width thinking row under the list in the chosen style, with the
  level meaning as a caption. The picker stays open after a model pick.
- Footer: speed left, access right, `flex-wrap gap-2`. No keyboard-hint cell;
  "Switching restarts the provider session" is a caption.
- Kept: favorites, accounts, search, pending and in-flight switching, Tab
  between zones, arrows, Enter, Escape, digit shortcuts in list style, aria.

Tests: updated `ChatModelControl.test.tsx` and
`ChatModelControl.columns.test.tsx`; selected row class differs from hover
class; strip arrow keys; the effort row sits after the list.

## Gates

`bun run check`, `NODE_OPTIONS=--no-experimental-webstorage bunx vitest run`,
`bun run build`. Screenshots (dark and light: General, one Agents & models
page, search highlight, model picker) in `BRIDGE_WORKER_OUTPUT_DIR`.
