# macOS screen and component audit

Updated September 7, 2026, with the chat revision below; the full screen pass was September 6, following the [macOS HIG reference](apple-macos-hig-reference.md). This extends the [initial foundation redesign](macos-frontend-redesign.md) with individual screen and component reviews. The redesign is implemented; this is not a claim that every runtime state or native macOS integration has passed visual validation. A shared token change alone does not count as a screen review.

The component inventory below records source review and implementation. The visual walkthrough records only screens actually opened. Provider-dependent and native-only states remain explicitly separate.

| Surface | Components and states | Status |
| --- | --- | --- |
| Conversation | Transcript, reasoning, tool groups, approvals, questions, messages, composer, attachments, file/agent/model/effort pickers | Reviewed and updated |
| Session inspectors | Dock, Changes and inline editor, Code and file picker, Terminal, Browser and connection states, Transcript, Tasks, GitHub lists and detail tabs | Reviewed and updated |
| Projects | Library, project lists, create/connect dialogs, workspace and orchestrator setup | Reviewed and updated |
| Marketplace | Agents, plugins and details, skills, automations, install/status/error states | Reviewed and updated |
| Settings | Appearance, permissions, composer, presets and preset editor, models, prompts, harnesses and details, work, import | Reviewed and updated |
| Memory | About me, pin editor/filter/actions, review queue and settings, Activity charts/budget/log, remember action | Reviewed and updated |
| Usage | Usage widget, provider details, login, context breakdown, health notices | Reviewed; existing shared health alert retained |
| Supplementary work | Aside chat, worker detail/steering, Work view, Mission Control | Reviewed and updated; existing route availability retained |
| Window and transient UI | Sidebar and filter menu, title bars, navigation, shortcuts, search/file palettes, toasts, shared menus/dialogs/controls | Reviewed and updated |
| First launch | Model setup wizard and role profile editor | Reviewed and updated |

## Hierarchy correction after the shared Safari screen

The supplied conversation screenshot exposed a real gap in the earlier pass: status and checkpoint cards competed with the conversation, metadata was too prominent, and repeated vertical gaps wasted the available height. The earlier “completed” wording overstated what source review alone established. This pass changes the layouts themselves and checks their rendered results.

| HIG principle from the reference | Concrete redesign | Rendered acceptance check |
| --- | --- | --- |
| Sustained work; spacing and grouping | Conversation reads in a 56rem column with 20px row gaps. The writing frame accounts for its own inset so composer and transcript align. | Populated conversation at 1440 × 900; composer and content both measured x=396 and width=896 CSS px. Compact 900 × 740 retains wrapped context controls. |
| Clear decisions; progressive disclosure | Adoption has an explicit merge/delete explanation, persistent actions, and expandable file/branch details. Verification is one status disclosure. Saved checkpoints remain separate, quiet transcript rows. Approval paths and the requested action stay visible; repeated policy instructions move into Policy. Repository divergence keeps the blocker visible and expands the diagnostic explanation. | Adoption, verification, checkpoint, compaction, branch, approval, worker approval, divergence, and expanded tool/diff rows inspected. No approval/adoption/discard executed in QA. |
| Related screens share structure | Shared `ui/screen` heading and page-width token govern Projects, Agents, Plugins, Skills, Automations, and Memory. Settings and Work use the same page geometry. Marketplace navigation occupies one wrapping toolbar. | All main destinations opened; page/detail alignment checked. |
| Content before supporting information | Projects use grouped repository/session lists with New agent beside each repository title. Installed plugins show names. Agent/plugin/skill/automation results use divided rows. Provider support follows automation schedules in a disclosure. | Project library; agents; plugin list/detail; expanded skill; automation list, support disclosure, and creation form. |
| Preserve context and work | Memory starts with search and saved entries; its editor follows the list. Editing still focuses and scrolls to the same editor. Review queue and Activity remain inside the one Memory screen. | All three Memory tabs; existing edit/supersession and prefilled-draft tests retained. |
| Readable controls; resize by available space | Message/UI/caption/title tokens are 15/13/12/24 CSS px. Compact shared text buttons have a 28px height; ordinary actions are 32px. Appearance previews use their settings container width instead of viewport breakpoints. The model picker intersects clipping ancestors and clamps to the actual available space, including after resize/scroll. | Wide Paper and compact Graphite walkthrough; labels and actions remain available. The repaired compact picker measured top=48, with its search field at y=58.75–78.25, below the 44px toolbar. These are Bridge choices, not a claim that native HIG point sizes map directly to CSS pixels. |
| Disclose technical detail when useful | Prompt preview exposes size first, with exact bytes, hashes, and provider layers in a disclosure. Cache-change feedback remains visible. Usage is a divided provider list, with concise unavailable-quota text. | Prompt page and expanded model controls; usage and context breakdown in a bounded popover. At 900 × 740, usage top=50, bottom=534, width=588 CSS px. |
| Keyboard and focus | Existing dialog trapping/restore, radio navigation, and dock roving tabs are preserved. Transcript tests now locate the content by a stable marker rather than a styling class. | Dock Home selects Changes; file search takes focus; creation dialogs cancel without creating a session. Automated grouping, scroll, focus, and cancellation assertions remain intact. |

### Latest walkthrough coverage

- **Paper, 1440 × 900:** conversation hierarchy; Projects; Memory saved list; Agents; Plugins and Notion detail; expanded skill; Automations; first-launch defaults and advanced profiles; all nine settings pages; Planner disclosure; research preset editor; Claude detail.
- **Graphite, 900 × 740:** Memory Review queue and Activity; every dock pane (Changes, Code, Terminal, Browser setup, Transcript, Tasks, GitHub); PR conversation; file palette and expanded editor/tree; usage and context breakdown; New project and isolated-worktree dialogs; automation creation dialog; model picker search; sidebar filter menu; chat search. Actions that would install, send, approve, merge, create worktrees, or save schedules were not used to obtain screenshots.
- **Retained coverage from the earlier pass:** GitHub checks/files/issues/repository screens, aside chat, router settings, other provider details, and the smallest settings picker. They retain their prior source and component-test coverage; this is not a claim every one was re-opened in this latest walkthrough.
- **Native validation:** the supplied Safari screenshot was reviewed, but automation could not reliably attach to its Bridge window. Packaged Tauri activation, traffic-light geometry, real terminal/browser host, VoiceOver, and OS accessibility preference propagation remain native validation tasks. Browser screenshots use the existing mock backend.

## Screen and component decisions

| Area and source components | Applied decisions |
| --- | --- |
| `App`, `AppTitleBar`, `SessionToolbar`, `BridgeSidebar`, `WindowNavButtons`, `SidebarFilterMenu` | Distinct navigation and content surfaces; visible screen/project titles; compact toolbar actions; selected rows and status words; keyboard resizing and collapse; consistent filter menu. Window navigation already met the control sizing and label requirements. |
| `ComposerPill`, `ComposerContextStrip`, `ChatModelControl`, `EffortList`, `EffortRail`, `EffortSlider`, `EffortSentence` | Legible editor, restrained controls, wrapping project/branch context. Model picker stacks its two sections when its container is narrow. Effort list supports arrow keys, Home/End, and one tab stop; existing rail keyboard behavior retained. Runtime-computed rail positions remain inline. |
| `AgentConversation`, `Markdown`, `DiffView`, `DiagramFigure` | Readable metadata, larger approval/question and copy controls, disabled question choices during submission. HTML previews use the shared modal and keep sandboxing. Diagram semantics and the common transcript grouping, reasoning, streaming, and scroll contracts retained. |
| `SessionDock`, `ui/pane` | Shared empty/loading/error presentation, compact tab strip, named panels, roving keyboard focus, arrow/Home/End tab selection. Divider exposes actual width and bounds; expand/restore keeps mounted pane state. |
| `ChangesPanel`, `InlineFileEditor` | File names occupy a full row in a narrow inspector; basename retained when a long prefix must truncate. File actions remain visible, Diff/Edit controls are larger, viewed state and stats use semantic tokens. Refresh errors have a retry action. Editor save/reload/conflict controls wrap; unsaved buffers and hash protection retained. |
| `CodePanel`, `CodeEditor` | Narrow panes prioritize the editor and expose Find a file; expanded panes reveal the tree. File palette uses a focus-managed modal, visible close controls, readable tree rows. CodeMirror gutter contrast improved; editing and save behavior retained. |
| `TerminalPane`, `TranscriptPane`, `TasksPane` | Consistent headers, readable metadata, visible tab close/retry/dismiss controls, wrapping status bars. Transcript filters and search use normal-sized fields; Tasks has a coherent empty state. Existing terminal sessions, event replay, and task logic retained. |
| `BrowserSurface` | Reviewed Setup, Connect, TabPicker, PageMirror, Elements, Timeline, Debug, and Metrics. Labeled fields, wrapping navigation/actions, readable connection and permission explanations, bounded tab list. Host setup and sensitive action behavior retained. |
| `GitHubPane`, `GithubToasts` | Reviewed PR/issue lists, repository overview, PR conversation/files/checks, issue details, labels, comment/review/checkout/merge confirmations. Larger compact actions, focus-managed confirmations, keyboard-operable PR tabs with associated panel, readable notification text and dismissal. Remote HTML remains inert. |
| `ProjectsScreen`, `NewProjectDialog`, `WorkspaceCreateDialog`, `OrchestratorCreateDialog`, `CreateDialogShell` | Grouped project lists and always-visible project actions; clear start-chat/folder choices; readable forms; right-aligned creation action. Dialogs trap Tab, restore focus, and block dismissal while busy. Cancel, Close, and Escape in orchestrator setup now cancel without creating a session; creation requires an explicit choice. |
| `MarketplaceScreen`, `AgentMarketplace`, `SkillMarketplace`, `AutomationsPanel` | Agents and plugins use divided lists; plugin details share the same page width and title hierarchy. Skills and automations use full-width expandable rows so explanations fit. Search fields have accessible labels; disclosures and filters expose selection; provider text wraps. Install, removal, and scheduling logic retained. |
| `SettingsScreen`, `SettingsRail`, settings `kit` | Stable section hierarchy, consistent Lucide icons, readable labels/help, wrapping field rows and footer actions, normal focus and validation states. Rail becomes a picker in the smallest windows. |
| `AppearancePage`, `PermissionsPage`, `ComposerPage` | System-following appearance, clear radio selection and keyboard operation, normal Graphite/Paper surfaces, grouped permissions, readable composer choices and help. |
| `PresetsPage`, `ModelsPage`, `ModelProfileEditor` | Compact lists and editor forms; model role editor uses container breakpoints; readable labels and model explanations. Save, discard, default, validation, disabled, and empty states reviewed. |
| `PromptStudio`, `ManagedAgentsPanel` | Prompt selection and editor use the shared hierarchy; code remains monospace. Managed-agent forms and confirmation use consistent materials, scroll bounds, and existing focus handling. |
| `HarnessesPage`, `OpenCodeHarnessSettings`, `WorkSettingsSection`, `ImportHarnessSection` | Reviewed provider list and detail forms, OpenCode choices, work briefing, and staged import controls/results. Shared field, help, status, and save patterns applied. Provider-specific authentication/import behavior retained. |
| `MemoryDialog`, `MemoryUsedChip` | One Memory screen: About me, Review queue, Activity. Pin controls and filters wrap; edit focuses the pin field; actions remain visible; packet budget, recall statistics, and consolidation log stay achromatic. Recalled text wraps in its popover. |
| `ContextBreakdown`, `HealthWarnings`, `BypassBadge` | Shared popover material, explicit unavailable measurements, compact actions, and persistent warning dismissal. The old `UsageWidget` ring and popover are retired; provider sign-in remains in `ProviderLoginPane` for setup and Harnesses settings. |
| `AsideChat`, `WorkerDetail`, `WorkView`, `MissionControl` | Aside uses a focus-managed modal; worker feed/steering text and controls are readable; Work hierarchy and fact/task/error states are consistent; Mission Control uses container-based columns. Existing hidden routes are not newly enabled. |
| `SessionRecallSearch`, `ShortcutsSheet`, `ModelSetupWizard`, `RouterSettingsDialog` | Legible search/shortcut rows; a compact first-run hierarchy with clear recommended and advanced paths; router settings use a bounded shared modal and readable grouped controls. |
| `ui/button`, `input`, `textarea`, `label`, `badge`, `tabs`, `dialog`, `menu-panel`, `pane`, `scroll-area`, `input-group`, `alert`, `kbd`, `spinner` | Shared control sizes, typography, semantic tokens, focus/disabled/error states. Menus and dialogs use the existing glass primitives. Scroll content uses a Tailwind min-width utility; alert, input-group, keyboard hint, and spinner mechanics reviewed and retained. |
| `BridgeMark`, `harnessMarks`, `connectorLogos` | Brand artwork retained; screen treatment is supplied by its surrounding controls. |

`WelcomeScreen.tsx` is an unused prototype with placeholder content; the shipped welcome view is owned by `App.tsx`. The prototype and development previews were not connected to navigation. `WorkspaceCreateDialog` is also currently unconnected; it was updated alongside the shared creation shell without enabling a second creation flow. Utility/data modules such as `workFacts`, `workTasks`, `workerStatus`, and editor buffer helpers retain their behavior and existing tests.

## Visual walkthrough

Browser checks used the existing Vite mock backend at `127.0.0.1:1420`. They verify rendered frontend behavior, not actual provider, filesystem, or GitHub writes.

| Surface | Browser observations |
| --- | --- |
| Welcome and conversation | Main shell, composer, context controls, populated transcript, tool/diff groups, waiting/working feedback, approval presentation. |
| Settings | All nine navigation pages: Appearance, Permissions, Composer, Presets, Models, Prompts, Harnesses, Work briefing, Import. Also preset editor, model role disclosure, prompt editor, Claude and OpenCode detail, import source selection. Later import stages and remaining provider-specific states covered by source review and tests. |
| Memory | About me, Review queue, Activity; pin text/actions and filters; checked in both theme/layout walkthroughs. |
| Marketplace | Agents, installed plugin detail, expanded skill, expanded automation, new automation dialog dismissed without saving. |
| Projects | Library, empty/nonempty project lists, New project choices, new orchestrator dialog. Native folder chooser not invoked. |
| Session panes | Changes file list and controls; Code empty/file search/open file/expanded tree; Terminal, Browser setup, Transcript, Tasks; GitHub PR list and three detail tabs, issue list/detail, repository overview. Connected browser states and real terminal I/O require native integration. |
| Usage | Provider unavailable states, show-more cache/history details, context breakdown and segment rows. Popover header verified at y=50 below the toolbar in both 1440 × 900 and 900 × 740 windows, with no document overflow. Login progress and other context variants covered by component tests. |
| Routing | Learning router modal inspected at 900 × 740: grouped policy fields, role profiles, bounded scrolling, persistent Cancel/Save footer. Dismissed without saving. |
| Resize | Wide 1440 × 900 preview and compact 900 × 740 window: Settings, Memory, dock sheet, retained open editor, and Changes labels. The initial foundation pass also covered the narrow section picker and collapsed sidebar. |

Both Paper and Graphite were visually inspected during the redesign. The final compact pass used Match macOS. Temporary browser viewport overrides are reset after verification.

## Checks and remaining native validation

September 6 validation passed (the September 7 chat checks are recorded below):

- `bun run build`: TypeScript and production Vite build passed after the final picker fix. The existing large-chunk advisory remains.
- `bun run test`: sidecar, frontend, and Rust stages passed sequentially. **38 sidecar tests passed, 1 skipped; 2,110 Rust tests passed, 13 ignored.**
- The final picker fix was followed by the complete frontend suite: **1,807 tests across 136 files passed.** It changes no native or sidecar source.
- `git diff --check`: passed.

Regression coverage retains dialog focus/restore and busy dismissal, dock tab navigation and resizing metadata, appearance/effort radio keyboard operation, GitHub tab/panel navigation, canceling orchestrator setup without creating a session, composer popover bounds, and transcript grouping/scroll behavior. The new model-picker test checks a clipping parent and a resize where neither side can fit the preferred minimum height; a preference for a taller menu no longer forces it outside the visible pane. Assertions and timeouts were not weakened.

Native window activation, traffic-light placement, real terminal/browser-host integration, wallpaper vibrancy, OS accessibility preference propagation, and VoiceOver were not verified in the browser. CSS materials are a webview interpretation of the reference, not AppKit Liquid Glass. Hidden Work/Mission Control surfaces and provider/error states that cannot be reached safely from the mock UI were checked through source review and existing component tests rather than claimed as browser-tested.


## Chat session revision — September 7, 2026

The September 6 implementation was not the final chat design. The user rejected the session's visual hierarchy in a new screenshot. This revision uses the same macOS reference, particularly comfortable sustained work, grouped content, discoverable toolbar commands, and readable status. It addresses the populated chat specifically.

| Component | Revised presentation |
| --- | --- |
| Conversation column | Transcript and composer share an 800 CSS-pixel maximum content width. User messages have a quiet filled surface; assistant prose remains on the canvas. Code scrolls inside its own region in narrow panes. |
| Activity group | One divided activity section replaces separate read labels, edit cards, command cards, and duration trailers. The header contains the summary, step count, and elapsed work. Chronological order, live state, manual disclosure choices, bounded large runs, and automatic inline patch visibility remain intact. Failed work and reported nonzero exits receive an explicit “Activity needs attention” label. |
| Tool rows and patches | Filename appears once, with the parent directory beside it. Full paths remain in tooltips and accessible Code-pane links. Output controls are at least 32 CSS pixels high. Inline patches retain line numbers, additions/deletions, internal scrolling, and remaining-hunk disclosure. |
| Startup feedback | Status text leads; elapsed time is secondary and uses minutes/hours for long waits. The screenshot's 42,163-second case renders as 11h 42m. Provider state, startup phases, and the streaming handoff are unchanged. |
| Composer | Smaller writing surface with a single controls row; project context sits below the field. Empty dock composer measures about 89 CSS pixels high, with a separate 28-pixel context line. Multiline drafts grow naturally. Stop and Queue/Steer remain distinct. |
| Project context | Project and branch labels share available width. Work mode and host become labeled icons in narrow composer containers; full wording remains in accessible names and tooltips. Context no longer becomes three stacked rows in a narrow pane. |
| Model control | The selected model remains readable while switching is unavailable during a response. Disabled semantics and the explanatory tooltip remain. |
| Changes action | The detached file-count pill is replaced with a labeled Review action in the session toolbar. It opens the Changes inspector and exposes its selected state. |

### Verification of this revision

- Inspected the populated session in Paper and Graphite: opening decisions, verification summary, user messages, checkpoint records, assistant prose, activity rows, inline diff, command output, startup status, and composer.
- Checked an effective 1309 × 818 CSS-pixel viewport, a 900 × 740 viewport, and the browser's normal narrow pane. The browser's existing zoom makes its requested viewport size differ from CSS viewport dimensions; measurements above come from the DOM.
- At the wide size, transcript and composer both measure 800 CSS pixels wide. At 900 × 740, both measure approximately 588 CSS pixels wide. The compact context row remains approximately 28 CSS pixels high, with no document horizontal overflow.
- Opened Review changes and verified the selected Changes pane. Checked activity collapse/reopen and command-output disclosure, including Enter activation.
- Entered and removed a two-line draft without submitting it: the composer grew to about 114 CSS pixels, Queue became available, and document overflow remained absent.
- Restored Match macOS and reset temporary viewport overrides after appearance/resize checks.
- Final `bun run build` passed. Final complete frontend run passed: **1,808 tests across 136 files**. Coverage includes cross-provider golden transcripts, large tool runs, scroll retention, live disclosure, Code-pane file links, composer behavior, and long elapsed-time formatting. `git diff --check` passed.

These are frontend checks against the local mock backend. The native/sidecar suite results recorded above belong to the September 6 pass; they were not rerun for this frontend-only revision. Native VoiceOver, window chrome, and actual provider execution remain outside this browser verification.

## Native integration follow-up — September 7, 2026

The approved redesign was subsequently integrated with current `main`. The production frontend build, native debug bundle, and complete test suite passed: **1,818 frontend tests, 2,189 Rust tests (13 ignored), and 44 sidecar tests (1 skipped)**.

Real Cursor Auto chats through the rebuilt native daemon completed two coding fixtures, approvals, a queued follow-up, recovery after a failed Claude request, and Stop followed by a new response. All six fixture tests passed on independent reruns, with only the requested source files changed. Claude Sonnet's live request failed on expired OAuth credentials. Native visual inspection remains pending because the desktop locked; GitHub Actions jobs could not start because of account billing. See [the native validation report](../testing/macos-redesign-native-validation.md) for the exact scope and evidence.

A subsequent installed-release restart check exposed stale provider process records after orderly shutdown. The cleanup now records a stopped session before exit and preserves existing terminal results. The regression reproduced the false failure state before the fix and passes afterward. Final full checks passed: **1,818 frontend tests, 2,190 Rust tests (13 ignored), and 44 sidecar tests (1 skipped)**; the production frontend build also passed.

## User refinements — September 7, 2026

The user requested the original dark appearance, a Projects grid, and an explanation for the inactive chat-context controls.

- Dark mode restores the original black canvas and sidebar, neutral raised surfaces, borders, text, code colors, and optional dark vibrancy material. The appearance preview and browser theme color agree with the restored palette. Paper, the revised chat layout, and OS contrast/transparency accommodations remain in place.
- Projects uses separate cards in one, two, or three columns according to the available content width. Each card keeps its project title, path, branch, change counts, chat links, and visible New agent/Connect folder actions.
- Project, branch, work mode, and host controls open accessible menus. Existing chats retain their checkout and explain that restriction, with an action to open a new draft for different settings. Draft work mode uses explicit menu choices. The host menu identifies This Mac as the available host and Cloud/SSH as unavailable; no remote-host implementation is implied.

Browser verification inspected the restored dark appearance and the Projects grid at requested viewport widths of 900, 1160, and 1440 pixels. These rendered one, two, and three columns respectively. Project chat links and New agent/Cancel worked. Existing-chat menus displayed their restrictions and opened a new draft without creating or retargeting a session. Draft work-mode selections changed the draft; host availability was explicit. Temporary viewport overrides were reset, and no browser console errors were recorded.

The production build and complete test command passed: **1,818 frontend tests, 2,190 Rust tests (13 ignored), and 44 sidecar tests (1 skipped)**. Component checks cover locked menu behavior, unsupported host choices, keyboard dismissal/focus, and work-mode selection. The App integration test verifies that choosing an isolated worktree creates nothing until submission and then passes `createWorktree: true` to session creation. Native inspection of this follow-up bundle requires the desktop to be unlocked.
