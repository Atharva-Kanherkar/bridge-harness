# Settings and first-run setup

Bridge uses **coding agent** for a tool such as Claude Code, Codex, Cursor, OpenCode, or Grok. A model is the AI model that tool uses. Bridge's internal roles are task instructions, not another coding agent to install.

On first launch, choose one or more coding agents. Bridge shows which installations it found and which are missing:

- **Your own install:** Bridge uses the copy already on your computer or at your chosen location. You manage its updates. Bridge cannot remove it.
- **Bridge-managed:** Bridge downloads a separate copy where a supported installation recipe exists. Bridge can update or remove that copy. You still use your provider account.
- **Not installed:** install with Bridge when offered, or follow the vendor's installation instructions. If Bridge has no supported download or repair, it says so.

The next step handles installation and sign-in. Every selected agent must have a usable installation, be available, and report signed-in before the workspace opens. Closing a sign-in pane does not establish success; Bridge checks again. Unknown sign-in status stays unresolved. You can return to agent selection to remove an unfinished choice.

For Cursor and Grok, Continue opens and closes a temporary session to check sign-in and read the models the agent actually reports. It sends no chat message, but the agent may start its configured tools. Bridge repeats this check before opening the workspace. Routine background discovery does not open sessions.

If installation detection fails, you can retry or choose **Use detected agents**. Bridge still checks availability and sign-in. It labels installation ownership as unknown and offers no managed-install actions until detection recovers.

Bridge checks local sign-in facts. It does not verify your subscription, credits, or access to every advertised model. Setup chooses model defaults from the selected agents' usable catalogs. A limited catalog can use the same actual model for several purposes; this does not give it additional capabilities. You can change model preferences later. Existing installations with saved Bridge data or completed setup retain their current onboarding state.

## Where settings live

| Destination | Controls |
| --- | --- |
| General | Appearance, typing and history search, menu bar, updates |
| Coding agents | Installation, sign-in, agent defaults, model preferences, agents |
| Permissions | Action approvals and browser copies |
| Data & storage | Import history and local disk cleanup |

Background task limits, internal role instructions, and specialized display controls live under Advanced. Existing links still open their corresponding detail pages. Built-in role setups remain editable through Advanced; the ordinary saved-setup list shows your own setups.

Agents are also beside New Chat. Archived chats and scheduled tasks open from the sidebar. Daily briefing settings open from the Work board.

## Changing settings from a chat

Any top-level chat can change some settings when you ask: "make the theme light", "hide thinking in chats", "make Sonnet my default model". The chat uses two tools on Bridge's own `bridge` tool server, `settings_read` and `settings_change`, on Claude, Codex and OpenCode alike.

| Key | Values | Applies to |
| --- | --- | --- |
| `appearance.theme` | system, light, dark | every open window, at once |
| `appearance.skin` | graphite, vibrancy, omarchy | every open window, at once |
| `appearance.effortSelector` | slider, sentence, list | every open window, at once |
| `transcript.showThinking` | true, false | every open window, at once |
| `transcript.autoExpandEdits` | true, false | every open window, at once |
| `newChat.model` | a harness and model, or default | new chats |
| `newChat.effort` | a level the default model supports, or default | new chats |
| `newChat.createWorktree` | true, false, or default | new chats |
| `newChat.activeTurnInput` | steer, queue, or default | new chats |

The list is closed on purpose. A chat cannot change default access, permissions, API keys, accounts, budgets, worker limits or approvals: an agent must never widen what it may do. Workers get neither tool. New-chat changes go through the same checks as [New chat defaults](chat-defaults.md), and only the application-wide defaults change; a project with its own value keeps it, and the chat says so.

Appearance and transcript choices are stored by the core (`config/get_app_preferences`, `config/save_app_preferences`) and announced with `config/app_preferences_changed`, so a change from a chat, Settings or another window reaches every window. Each window keeps a copy in localStorage only to paint its first frame. The live eval is `settings_tools_eval` in bridge-core; the test contract is [feat-bridge-control-mcp](../testing/feat-bridge-control-mcp.md).

## Marketplace

**Apps** connect services and tools to supported coding agents. **Skills** add reusable instructions and workflows. Coding-agent installation lives in Settings, under Coding agents. Scheduled tasks have their own sidebar destination.

The test contract and real-provider review steps are in [the setup test contract](../testing/feat-simple-settings-onboarding.md).
