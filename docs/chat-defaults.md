# Personal new-chat defaults

Settings > Coding agents > New chat defaults stores personal application defaults
and overrides keyed by Bridge project ID. A workspace resolves its project through
its database row. Preferences never come from repository files, a project name,
a path prefix, a prompt, or a Git remote.

A draft holds only explicit edits. The shared core resolves chat edits over project
preferences over application preferences over the existing agent/model defaults.
Selecting another project re-resolves inherited fields and preserves explicit edits
and unsent text. False, no selected agent, and provider-default model/effort are
explicit choices. Removing a field restores inheritance.

Every new ordinary chat uses `sessions/create_configured_chat`, which returns its
exact ID. The host checks the reviewed defaults revision before preparing a
worktree and again inside the insertion transaction. The complete configuration
is durable before a provider starts. Opening Settings or a draft creates no chat.
Existing sessions retain their snapshot when defaults change or are reset.

## Controls

| Control | Default and override behavior |
| --- | --- |
| Direct / Orchestrator / enabled specialist | Application/project default; explicit draft selection. A specialist uses the existing host dispatch and role/path policy. |
| Harness and model | Stable runtime/model IDs, live compatibility checks, explicit provider-default option. |
| Thinking effort | Provider-advertised string, bounded and validated against the chosen model. Provider default is distinct from inheritance. Specialist dispatch also follows its existing role effort contract. |
| User approval / Full access | Scoped to this chat and its worker family; never writes the global permission toggle when a draft or chat is edited. |
| Current checkout / isolated worktree | Defaulted before first submit. Worktrees are prepared only on submit and cleaned up on failed persistence. |
| Steer / Queue | Saved per project and snapshotted per chat; providers without steer use the existing safe fallback. |
| Instructions | Select an existing agent ID. Instructions remain in the existing preset/prompt surfaces. |
| Project, branch, checkout identity | Transient execution context. Active processes keep their checkout. Use a continuation/new chat to change context. No branch operation is stored as a preference. |
| Agent/mode after a conversation begins | Chat settings offers a visible continuation with handoff. A running provider is never relabeled as another mode. |
| Worker notices | Existing global/session notification controls keep their independent delivery semantics. |
| Browser, microphone, connector or credential grants | Independent grants, never defaulted by this feature. |
| Message text, attachments, pending approvals, worker IDs | Transient, never exported by Save as defaults. |
| Theme, layout, typing suggestions, voice setup | Existing application preferences; not execution settings. |

Chat settings exposes explicit Save as this project's defaults and Save as
application defaults actions. Editing a model or sending a message does not save
preferences. A draft whose reviewed defaults became stale refuses the send,
refreshes its choices, and keeps the user's text for review and retry.

## Access boundaries

These are the two existing approval postures, not a new four-mode permission UI.
User approval passes an explicit workspace-write mode to supported adapters;
Full access passes the existing unrestricted mode. Unsupported combinations fail
before startup, including approval-mode Pi sessions. A worktree alone is not a
sandbox, and approval policy is separate from filesystem/network containment.
Provider-default model and effort remain unpinned on configured starts/resumes.
Unavailable pinned models are diagnosed rather than silently substituted.

Workers resolve authority through actual session ancestry. An approval-mode parent
cannot request an unrestricted worker. Browser outward effects, prompt mutation
and delegation path grants keep their separate gates. Changing access acquires
lifecycle claims, cancels queued delegation and pending permissions, retires the
family's runtimes, and clears native thread IDs that could carry standing grants.
The next turn restores from Bridge's history under the new authority. Forks and
asides copy source authority transactionally into their independent root snapshots.

Legacy sessions keep their explicitly existing global-policy behavior until the
user changes that chat's options. New chats start with a scoped User approval
snapshot unless the user explicitly configures another posture. Resetting
preferences retains existing session authority. Malformed security snapshots fail
closed. No credentials, raw provider arguments or executable paths belong in the
preference schema.

Protocol 1.23 requires matching clients/daemons so a client cannot present a global
access label for a session that actually runs under scoped authority. Generated
artifacts and Tauri/daemon signature parity are tested. The broader read-only,
guarded and workspace permission taxonomy is separate work.
