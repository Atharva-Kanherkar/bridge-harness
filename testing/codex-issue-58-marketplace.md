# codex/issue-58-marketplace — Test Contract

## Functional Behavior

- Bridge lists available and installed Codex and Claude plugin variants through provider CLI adapters without reading provider credential stores.
- Provider variants group into one service only through explicit aliases or matching repository/publisher, remote MCP endpoint, or verified package metadata; display-name equality alone never merges variants.
- Users can install for Codex, Claude, or both and can enable, disable, update, uninstall, retry, or start provider-native authentication independently per variant.
- A dual-provider install reports each provider result, preserves partial success, and allows retrying only the failed provider.
- Installation, enablement, and authentication remain distinct states. Remote OAuth defaults to separate provider logins; shared authentication is shown only when provider metadata explicitly declares a supported external credential mechanism.
- The UI does not render an `Auth unknown` state. Authentication status is omitted when a provider reports neither `connected` nor `required`; provider-native auth controls appear only when login is explicitly required.
- Codex plugins that declare an app in `.app.json` are treated as app connectors, not named MCP servers. Bridge never runs `codex mcp login <plugin-name>` for them; it routes authentication to the native Codex/ChatGPT plugin surface with an actionable message.
- Installing a Codex plugin uses the signed-in Codex app-server `plugin/install` operation so Codex materializes the plugin in its own app/plugin state. Bridge does not treat a CLI cache copy alone as successful app installation.
- When `plugin/install` reports `appsNeedingAuth`, Bridge opens the provider-owned `installUrl` returned by Codex. Bridge never logs, persists, rewrites, or proxies authorization URLs or credentials, and the user completes consent in the browser.
- Bridge refreshes installed Codex app accessibility through the experimental app-server `app/list` operation. `isAccessible=true` maps to connected, `false` with an install URL maps to needs login, and missing/unavailable remote state remains unreported rather than guessed.
- Named remote MCP variants continue to use `codex mcp login <server-name>`. Authentication actions must fail safely when Bridge cannot identify a supported native authentication route.
- Installing a Claude plugin uses `claude plugin install <plugin@marketplace>` and Bridge refreshes the Claude catalog from `claude plugin list --available --json`; a successful cache install is not treated as connector authentication.
- Bridge discovers MCP servers contributed by installed Claude plugins from the CLI catalog and addresses them by Claude's canonical namespaced ID (`plugin:<plugin-name>:<server-name>`). It also lists configured native `claude.ai …` connectors reported by `claude mcp list` even when they are not marketplace plugins.
- Claude connector authentication uses `claude mcp login <canonical-server-id>`, allowing Claude Code to open and complete its provider-owned OAuth flow. Bridge never invokes the interactive `/mcp` slash command as a process and never reads, stores, rewrites, or proxies Claude OAuth credentials or authorization URLs.
- Bridge refreshes Claude connector state from `claude mcp list`: only explicit `Connected` and `Needs authentication` results map to connected/required. Unknown, malformed, timed-out, or unavailable status remains unreported rather than guessed.
- Bridge discovers enabled Claude plugin installation paths and configured native `claude.ai …` connector endpoints through credential-free CLI output, then passes those explicit local-plugin and remote-MCP definitions to the Claude Agent SDK. The sidecar continues loading `project` and `local` setting sources but does not inherit unrelated global user hooks or permissions. Provider credentials remain owned and resolved by Claude Code. Existing running sessions may require restart because plugin loading occurs at SDK session initialization.
- Native variants are preferred over convertible MCP variants. Provider-specific connectors, hooks, skills, agents, or apps are never marked portable without an explicit mapping.
- Command failures are actionable but redact likely secrets from all surfaced output. Marketplace sources remain visible before installation.
- Existing supervision, session persistence, permission-mode mapping, and Codex runtime behavior remain unchanged; Claude SDK initialization additionally inherits the trusted desktop user's enabled plugins and connectors.
- The marketplace uses a compact Codex-style information hierarchy: a concise title and search field, an installed-plugin icon strip, Public/Personal catalog tabs, and a dense two-column plugin list. Provider-specific install, lifecycle, and authentication controls remain available without turning every catalog entry into a large card.
- Plugin rows and the installed strip use the provider-owned logo declared by a local Codex or Claude plugin manifest when available. Logo paths must stay within the plugin directory, supported files are size-capped, and initials remain the fallback when no validated local asset exists.

## Unit Tests

- Rust catalog parsing accepts common JSON envelopes and retains provider metadata.
- Rust command construction uses provider-native CLI operations and never supplies credentials in arguments.
- Rust authentication routing distinguishes Codex app connectors from named MCP servers and never constructs an MCP login command for an app connector.
- Rust app-server request handling performs the initialize/initialized handshake, uses `plugin/install` with the selected marketplace, extracts authorization URLs without surfacing query parameters, and terminates the helper process on success, error, or timeout.
- Rust app accessibility parsing maps connector IDs to explicit connected/required states without reading credential stores.
- Rust Claude catalog parsing derives canonical namespaced connector IDs from nested `mcpServers` metadata and creates native variants for configured `claude.ai …` connectors.
- Rust Claude MCP status parsing accepts only explicit connected/required health lines, discards endpoint text, and associates status with the exact canonical server ID.
- Rust Claude authentication routing constructs `claude mcp login <canonical-server-id>` for plugin MCP servers and native connectors, never `claude /mcp …` and never an unqualified plugin name.
- Rust error sanitization redacts token-, secret-, authorization-, and key-shaped values.
- Rust logo resolution accepts supported manifest image assets inside the plugin root and rejects traversal, unsupported formats, and oversized files.
- Agent SDK option tests require `settingSources: ["project", "local"]`, explicit enabled local-plugin paths, explicit credential-free native connector endpoint definitions, `strictMcpConfig: false`, and no credential-bearing MCP fields.
- TypeScript catalog grouping follows the confidence order and refuses name-only matches.
- TypeScript compatibility classification defaults remote OAuth to separate login and recognizes only explicit shared mechanisms.
- TypeScript authentication presentation returns labels only for explicit connected/required states and hides unknown or unrecognized states.
- TypeScript dual-install orchestration preserves per-provider success/failure and retries only failures.

## Integration / Functional Tests

- The Tauri marketplace commands expose catalog refresh and per-provider lifecycle actions with structured results.
- The React marketplace renders the fast provider catalog first, then asynchronously enriches Codex app and Claude connector authentication state without overlapping refresh requests.
- The React marketplace loads catalog data, filters/searches grouped services, and invokes provider actions through the API boundary.
- Installed services are separated from the discoverable catalog, catalog tabs and provider filters compose with search, and service rows remain keyboard-accessible while exposing provider detail controls on demand.
- Existing API, conversation, observability, usage, sidecar, and Rust tests remain green.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.
- `bun run check` completes successfully.
- With provider CLIs unavailable or returning an error, the marketplace remains usable and shows independent provider errors.

## E2E Tests

N/A — provider marketplace and native authentication flows require locally installed, authenticated interactive CLIs. The UI/API boundary and orchestration are covered by automated functional tests; native prompt completion is verified manually.

## Manual / cURL Tests

- Run `bun run tauri dev`, open Marketplace, and verify Codex and Claude availability/errors render independently.
- Search and provider filters update the service cards without losing provider state.
- Verify the installed icon strip and two-column Public catalog remain readable at desktop width and collapse cleanly to one column at narrow width.
- Choose each install target and confirm the UI displays distinct install, enablement, and authentication results.
- Trigger **Connect Codex** for a named MCP server and confirm Bridge starts `codex mcp login` without displaying credential material.
- Install an app-backed Codex plugin such as Vercel and confirm it appears in Codex's installed plugin state, Bridge opens the exact provider-owned authorization page returned by Codex, and no authorization URL appears in Bridge logs/results.
- Complete provider consent in the browser, return to Bridge, and confirm the asynchronous app-state refresh changes the connector to **Connected** without reading or storing provider credentials.
- Trigger **Connect** again for an unauthenticated app connector and confirm Bridge requests a fresh install URL from Codex instead of running `codex mcp login vercel` or merely opening the ChatGPT home screen.
- Install a Claude plugin that contributes MCP, such as Vercel, and confirm Bridge shows both the installed plugin and its explicit connector status from `claude mcp list`.
- Trigger **Connect Claude Code** and confirm Bridge runs `claude mcp login plugin:vercel:vercel`, Claude opens its native browser flow, and Bridge never displays credential material or runs `claude /mcp vercel`.
- Confirm configured `claude.ai …` connectors appear as Claude connector variants with explicit status and can launch `claude mcp login <exact connector name>` when authentication is required.
- Start a fresh Bridge Claude session after enabling a plugin/connector and confirm the Agent SDK initialization reports the plugin and MCP server; an already-running session may be restarted to reload initialization-time components.
- Force one side of a dual install to fail and confirm the successful side remains installed while the failed side alone offers retry.
