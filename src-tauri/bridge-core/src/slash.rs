//! Slash-command catalog for Bridge chats.
//!
//! Combines provider builtins (Claude Code, Codex, and OpenCode) with user-installed
//! commands/skills. Direct chats auto-route
//! to the command's harness; skills/custom prompts are expanded into the
//! turn text so they work outside each provider's TUI.

use crate::capability_projection::{
    command_discovery_roots, skill_discovery_roots, CapabilityHarness,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub harness: String,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub enum SlashDispatch {
    /// Send `text` to the provider as-is (already harness-matched).
    Forward { text: String },
    /// Replace the turn with an expanded skill/prompt body.
    Expand { text: String },
    /// Bridge-handled: refresh account usage for the session harness.
    Usage,
    /// Composer-owned dictation entry point.
    Voice,
    /// Bridge-routed: ask the harness to compact its own context, falling
    /// back to a Bridge checkpoint only where the harness has no compaction
    /// command. `focus` is forwarded where the harness accepts one and
    /// reported as not applied where it does not.
    Compact { focus: Option<String> },
    /// Bridge-handled: clear provider session / start fresh in this chat.
    Clear,
    /// Bridge-handled: FTS5 recall in this session only.
    Recall { query: String },
    /// Bridge-handled: save an about-me pin to `account:local`.
    Pin { body: String },
    /// Bridge-handled: list active `account:local` pins.
    Pins,
    /// Bridge-handled: tombstone an `account:local` pin.
    Unpin { selector: String },
    /// Bridge-handled: open a side chat beside this conversation. The side
    /// chat reads the parent's projected context in its own session and never
    /// appends to the parent; the composer intercepts the command, so this arm
    /// only answers clients that submit it directly.
    SideChat { command: String, query: String },
    /// Known TUI-only command — tell the user it isn't available here.
    Unsupported { name: String, harness: String },
}

pub fn list_commands(available: &std::collections::HashSet<String>) -> Vec<SlashCommand> {
    list_commands_for_project(available, None)
}

pub fn list_commands_for_project(
    available: &std::collections::HashSet<String>,
    project: Option<&Path>,
) -> Vec<SlashCommand> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut out: Vec<SlashCommand> = vec![
        SlashCommand {
            name: "voice".into(),
            description: "Dictate text into the composer (Codex, when supported)".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
        SlashCommand {
            name: "btw".into(),
            description: "Open a side chat that reads this chat's context without touching it".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
        SlashCommand {
            name: "recall".into(),
            description: "Search this chat's history (this session only)".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
        SlashCommand {
            name: "pin".into(),
            description: "Save an about-me pin on this machine (account:local)".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
        SlashCommand {
            name: "pins".into(),
            description: "List this machine's about-me pins".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
        SlashCommand {
            name: "side".into(),
            description: "Open a side chat that reads this chat's context without touching it".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
        SlashCommand {
            name: "unpin".into(),
            description: "Forget an about-me pin by id".into(),
            harness: "bridge".into(),
            kind: "builtin".into(),
        },
    ];

    if available.contains("claude") {
        out.extend(claude_builtins());
        for root in command_discovery_roots(CapabilityHarness::Claude, &home, project) {
            for (name, path) in collect_md_commands(&root) {
                out.push(SlashCommand {
                    name,
                    description: read_md_description(&path),
                    harness: "claude".into(),
                    kind: "command".into(),
                });
            }
        }
        for root in skill_discovery_roots(CapabilityHarness::Claude, &home, project) {
            for (name, path) in collect_skills(&root) {
                out.push(SlashCommand {
                    name,
                    description: read_md_description(&path),
                    harness: "claude".into(),
                    kind: "skill".into(),
                });
            }
        }
    }

    if available.contains("codex") {
        out.extend(codex_builtins());
        for root in command_discovery_roots(CapabilityHarness::Codex, &home, project) {
            for (name, path) in collect_md_commands(&root) {
                out.push(SlashCommand {
                    name,
                    description: read_md_description(&path),
                    harness: "codex".into(),
                    kind: "prompt".into(),
                });
            }
        }
        for root in skill_discovery_roots(CapabilityHarness::Codex, &home, project) {
            for (name, path) in collect_skills(&root) {
                out.push(SlashCommand {
                    name,
                    description: read_md_description(&path),
                    harness: "codex".into(),
                    kind: "skill".into(),
                });
            }
        }
    }

    if available.contains("opencode") {
        out.extend(opencode_builtins());
        for root in skill_discovery_roots(CapabilityHarness::OpenCode, &home, project) {
            for (name, path) in collect_skills(&root) {
                out.push(SlashCommand {
                    name,
                    description: read_md_description(&path),
                    harness: "opencode".into(),
                    kind: "skill".into(),
                });
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    out.retain(|command| {
        seen.insert((command.harness.clone(), command.name.to_ascii_lowercase()))
    });
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.harness.cmp(&b.harness))
    });
    out
}

/// Provider-owned memory commands in the current catalog. Derived from the
/// catalog itself — an unavailable adapter contributes nothing, and no harness
/// name is compared here, only command names the catalog already carries.
pub fn provider_memory_commands(
    available: &std::collections::HashSet<String>,
) -> Vec<SlashCommand> {
    list_commands(available)
        .into_iter()
        .filter(|command| {
            command.harness != "bridge" && matches!(command.name.as_str(), "memory" | "memories")
        })
        .collect()
}

/// Parse a leading `/name …` turn and decide how Bridge should handle it.
pub fn dispatch(
    text: &str,
    session_harness: &str,
    available: &std::collections::HashSet<String>,
) -> SlashDispatch {
    dispatch_for_project(text, session_harness, available, None)
}

pub fn dispatch_for_project(
    text: &str,
    session_harness: &str,
    available: &std::collections::HashSet<String>,
    project: Option<&Path>,
) -> SlashDispatch {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return SlashDispatch::Forward {
            text: text.to_string(),
        };
    };
    let mut parts = rest.splitn(2, char::is_whitespace);
    let Some(name) = parts.next().filter(|value| !value.is_empty()) else {
        return SlashDispatch::Forward {
            text: text.to_string(),
        };
    };
    let args = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match name.to_ascii_lowercase().as_str() {
        "voice" => return SlashDispatch::Voice,
        "btw" | "side" => {
            return SlashDispatch::SideChat {
                command: name.to_ascii_lowercase(),
                query: args.unwrap_or("").to_string(),
            };
        }
        "usage" | "cost" | "stats" => return SlashDispatch::Usage,
        "recall" => {
            return SlashDispatch::Recall {
                query: args.unwrap_or("").to_string(),
            };
        }
        "pin" => {
            return SlashDispatch::Pin {
                body: args.unwrap_or("").to_string(),
            };
        }
        "pins" => return SlashDispatch::Pins,
        "unpin" => {
            return SlashDispatch::Unpin {
                selector: args.unwrap_or("").to_string(),
            };
        }
        "compact" => {
            return SlashDispatch::Compact {
                focus: args.map(str::to_string),
            };
        }
        "clear" | "new" | "reset" => return SlashDispatch::Clear,
        _ => {}
    }

    let catalog = list_commands_for_project(available, project);
    let matches: Vec<_> = catalog
        .iter()
        .filter(|command| command.name.eq_ignore_ascii_case(name))
        .collect();
    if matches.is_empty() {
        // Unknown slash — still forward so the provider can try.
        return SlashDispatch::Forward {
            text: text.to_string(),
        };
    }

    // Prefer the session harness when the same name exists in both.
    let chosen = matches
        .iter()
        .find(|command| command.harness == session_harness)
        .or_else(|| matches.first())
        .map(|command| (*command).clone())
        .expect("matches non-empty");

    if chosen.harness != session_harness {
        // Caller must switch harness before re-dispatching. We still return
        // Forward with original text so the UI can switch + resend.
        return SlashDispatch::Forward {
            text: text.to_string(),
        };
    }

    match chosen.kind.as_str() {
        "skill" | "command" | "prompt" => {
            if let Some(body) = load_expandable_body(&chosen.harness, &chosen.name, project) {
                let mut expanded = format!("# /{}\n\n{}\n", chosen.name, body.trim());
                if let Some(args) = args {
                    expanded.push('\n');
                    expanded.push_str(args);
                    expanded.push('\n');
                }
                return SlashDispatch::Expand { text: expanded };
            }
            SlashDispatch::Forward {
                text: text.to_string(),
            }
        }
        "builtin" => {
            if is_forwardable_builtin(&chosen.harness, &chosen.name) {
                SlashDispatch::Forward {
                    text: text.to_string(),
                }
            } else {
                SlashDispatch::Unsupported {
                    name: chosen.name.clone(),
                    harness: chosen.harness.clone(),
                }
            }
        }
        _ => SlashDispatch::Forward {
            text: text.to_string(),
        },
    }
}

/// True when Bridge handles the slash locally and must never auto-switch harness.
pub fn is_bridge_local(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "btw" | "side"
            | "usage"
            | "cost"
            | "stats"
            | "compact"
            | "clear"
            | "new"
            | "reset"
            | "recall"
            | "pin"
            | "pins"
            | "unpin"
    )
}

/// Builtins that are meaningful as a normal provider turn (skills-like or
/// prompt-mode commands). Pure TUI chrome stays unsupported.
fn is_forwardable_builtin(harness: &str, name: &str) -> bool {
    const CLAUDE: &[&str] = &[
        "init",
        "plan",
        "review",
        "code-review",
        "security-review",
        "simplify",
        "memory",
        "doctor",
        "debug",
        "insights",
        "recap",
        "batch",
        "loop",
        "verify",
        "run",
        "deep-research",
        "claude-api",
        "dataviz",
        "design-sync",
    ];
    const CODEX: &[&str] = &[
        "init", "plan", "review", "diff", "mention", "goal",
    ];
    match harness {
        "claude" => CLAUDE.iter().any(|value| *value == name),
        "codex" => CODEX.iter().any(|value| *value == name),
        "opencode" => matches!(
            name,
            "models" | "sessions" | "new" | "undo" | "redo" | "share" | "help"
        ),
        _ => false,
    }
}

fn load_expandable_body(harness: &str, name: &str, project: Option<&Path>) -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let capability_harness = CapabilityHarness::from_id(harness)?;
    let mut candidates = command_discovery_roots(capability_harness, &home, project)
        .into_iter()
        .map(|root| root.join(format!("{name}.md")))
        .collect::<Vec<_>>();
    candidates.extend(
        skill_discovery_roots(capability_harness, &home, project)
            .into_iter()
            .map(|root| root.join(name).join("SKILL.md")),
    );
    if name.contains(':') {
        let relative = name.replace(':', "/");
        candidates.extend(
            command_discovery_roots(capability_harness, &home, project)
                .into_iter()
                .map(|root| root.join(format!("{relative}.md"))),
        );
        candidates.extend(
            skill_discovery_roots(capability_harness, &home, project)
                .into_iter()
                .map(|root| root.join(&relative).join("SKILL.md")),
        );
    };
    for path in candidates {
        if path.is_file() {
            return std::fs::read_to_string(path).ok();
        }
    }
    None
}

fn read_md_description(path: &Path) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if content.trim_start().starts_with("---") {
        for line in content.lines().skip(1) {
            let trimmed = line.trim();
            if trimmed == "---" {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("description:") {
                let value = rest.trim().trim_matches('"').trim_matches('\'').trim();
                if !value.is_empty() {
                    return value.chars().take(140).collect();
                }
            }
        }
    }
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "---" || trimmed.starts_with("description:") {
            continue;
        }
        let text = trimmed.trim_start_matches('#').trim();
        if !text.is_empty() {
            return text.chars().take(140).collect();
        }
    }
    String::new()
}

fn collect_md_commands(base: &Path) -> Vec<(String, PathBuf)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if path.extension().and_then(|value| value.to_str()) == Some("md") {
                if let Ok(relative) = path.strip_prefix(base) {
                    let name = relative
                        .with_extension("")
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join(":");
                    if !name.is_empty() {
                        out.push((name, path));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(base, base, &mut out);
    out
}

fn collect_skills(base: &Path) -> Vec<(String, PathBuf)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                let skill = path.join("SKILL.md");
                if skill.is_file() {
                    if let Ok(relative) = path.strip_prefix(base) {
                        let key = relative
                            .components()
                            .map(|component| component.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join(":");
                        if !key.is_empty() {
                            out.push((key, skill));
                        }
                    }
                } else {
                    walk(base, &path, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(base, base, &mut out);
    out
}

fn builtin(name: &str, description: &str, harness: &str) -> SlashCommand {
    SlashCommand {
        name: name.into(),
        description: description.into(),
        harness: harness.into(),
        kind: "builtin".into(),
    }
}

fn claude_builtins() -> Vec<SlashCommand> {
    [
        ("add-dir", "Add a working directory for this session"),
        ("advisor", "Enable or disable the advisor tool"),
        ("agents", "Create or manage subagents"),
        (
            "autofix-pr",
            "Watch the branch PR and fix CI/review comments",
        ),
        ("background", "Detach this session as a background agent"),
        ("batch", "Fan out a large change across parallel worktrees"),
        (
            "branch",
            "Branch the conversation to try a different direction",
        ),
        ("cd", "Move this session to a new working directory"),
        ("chrome", "Configure Claude in Chrome"),
        ("claude-api", "Load Claude API reference material"),
        ("clear", "Start a fresh conversation (keep project memory)"),
        (
            "code-review",
            "Review the current diff for bugs and cleanups",
        ),
        ("color", "Set the prompt bar color"),
        ("compact", "Summarize the conversation to free context"),
        ("config", "Open or set Claude Code settings"),
        ("context", "Visualize current context usage"),
        ("copy", "Copy the last assistant response"),
        ("cost", "Alias for /usage"),
        ("dataviz", "Design guidance for charts and dashboards"),
        ("debug", "Enable debug logging and troubleshoot issues"),
        ("deep-research", "Fan out web research into a cited report"),
        ("design-login", "Authorize design-system access"),
        (
            "design-sync",
            "Sync your React design system to Claude Design",
        ),
        ("desktop", "Continue this session in Claude Desktop"),
        ("diff", "Show uncommitted and per-turn diffs"),
        ("doctor", "Run a setup checkup and fix issues"),
        ("effort", "Set model effort level"),
        ("exit", "Exit the CLI"),
        ("export", "Export the conversation as plain text"),
        ("fast", "Toggle fast mode"),
        ("feedback", "Submit feedback or report a bug"),
        (
            "fewer-permission-prompts",
            "Auto-allow common read-only tools",
        ),
        ("focus", "Toggle focus view"),
        ("fork", "Spawn a forked subagent from this conversation"),
        ("goal", "Set a persistent goal across turns"),
        ("help", "Show help and available commands"),
        ("hooks", "View hook configurations"),
        ("ide", "Manage IDE integrations"),
        ("init", "Initialize a CLAUDE.md for the project"),
        (
            "insights",
            "Generate a report from your Claude Code sessions",
        ),
        ("install-github-app", "Install the Claude GitHub App"),
        ("install-slack-app", "Install the Claude Slack app"),
        ("keybindings", "Open keyboard shortcuts"),
        ("login", "Sign in to Anthropic"),
        ("logout", "Sign out of Anthropic"),
        ("loop", "Run a prompt repeatedly on a schedule"),
        ("mcp", "Manage MCP server connections"),
        ("memory", "Edit CLAUDE.md memory files"),
        ("mobile", "Show QR code for the Claude mobile app"),
        ("model", "Switch the AI model"),
        ("passes", "Share a free week of Claude Code"),
        ("permissions", "Manage tool permission rules"),
        ("plan", "Enter plan mode"),
        ("plugin", "Manage Claude Code plugins"),
        ("powerup", "Interactive Claude Code feature lessons"),
        ("privacy-settings", "View and update privacy settings"),
        ("recap", "One-line summary of the current session"),
        ("release-notes", "View the changelog"),
        ("reload-plugins", "Reload active plugins"),
        ("reload-skills", "Re-scan skill and command directories"),
        ("remote-control", "Make this session remotely controllable"),
        ("remote-env", "Choose the default cloud agent environment"),
        ("rename", "Rename the current session"),
        ("resume", "Resume a conversation by id or name"),
        ("review", "Fast read-only review of a GitHub PR"),
        ("rewind", "Rewind conversation and/or code to a checkpoint"),
        ("run", "Launch and drive the project app to verify a change"),
        (
            "run-skill-generator",
            "Teach /run how to drive this project",
        ),
        ("sandbox", "Toggle sandbox mode"),
        ("schedule", "Create or manage cloud routines"),
        (
            "security-review",
            "Review the branch diff for security issues",
        ),
        ("simplify", "Cleanup review that applies fixes"),
        ("skills", "List available skills"),
        ("stats", "Alias for /usage"),
        ("status", "Show version, model, account, and connectivity"),
        ("statusline", "Configure the status line"),
        ("stickers", "Order Claude Code stickers"),
        ("stop", "Stop the attached background session"),
        ("tasks", "View and manage background work"),
        ("team-onboarding", "Generate a team onboarding guide"),
        ("teleport", "Pull a Claude Code on the web session here"),
        ("terminal-setup", "Configure terminal keybindings"),
        ("theme", "Change the color theme"),
        ("upgrade", "Open the upgrade page"),
        ("usage", "Show session cost and plan usage limits"),
        ("usage-credits", "Configure usage credits"),
        ("verify", "Build and observe the app to confirm a change"),
        ("voice", "Toggle voice dictation"),
        ("web-setup", "Connect GitHub to Claude Code on the web"),
        ("workflows", "Open the workflow progress view"),
    ]
    .into_iter()
    .map(|(name, description)| builtin(name, description, "claude"))
    .collect()
}

fn codex_builtins() -> Vec<SlashCommand> {
    [
        ("agent", "Switch the active agent thread"),
        ("app", "Continue in the ChatGPT desktop app"),
        (
            "approve",
            "Approve one retry of a recent auto-review denial",
        ),
        ("apps", "Browse apps/connectors and insert them"),
        ("archive", "Archive the current session and exit"),
        ("clear", "Clear the terminal and start a fresh task"),
        ("compact", "Summarize the conversation to free tokens"),
        ("copy", "Copy the latest completed Codex output"),
        ("debug-config", "Print config layer diagnostics"),
        ("delete", "Permanently delete the current session"),
        ("diff", "Show the git diff including untracked files"),
        ("exit", "Exit the CLI"),
        ("experimental", "Toggle experimental features"),
        ("fast", "Toggle the Fast service tier"),
        ("feedback", "Send logs to Codex maintainers"),
        ("fork", "Fork the current task into a new task"),
        ("goal", "Set, edit, pause, resume, or clear a goal"),
        ("hooks", "View and manage lifecycle hooks"),
        ("ide", "Include open IDE files and selection"),
        ("import", "Import Claude Code setup into Codex"),
        ("init", "Generate an AGENTS.md scaffold"),
        ("keymap", "Remap TUI keyboard shortcuts"),
        ("logout", "Sign out of Codex"),
        ("mcp", "List configured MCP tools"),
        ("memories", "Configure memory use and generation"),
        ("mention", "Attach a file to the conversation"),
        ("model", "Choose the active model"),
        ("new", "Start a new task in the same CLI session"),
        ("permissions", "Set what Codex can do without asking"),
        ("personality", "Choose a communication style"),
        ("pets", "Choose or hide a terminal pet"),
        ("plan", "Switch to plan mode"),
        ("plugins", "Browse installed and discoverable plugins"),
        ("ps", "Show background terminals"),
        ("quit", "Exit the CLI"),
        ("raw", "Toggle raw scrollback mode"),
        ("rename", "Rename the current task"),
        ("resume", "Resume a saved conversation"),
        ("review", "Ask Codex to review the working tree"),
        (
            "sandbox-add-read-dir",
            "Grant sandbox read access to a directory",
        ),
        ("setup-default-sandbox", "Set up the elevated Windows sandbox"),
        ("skills", "Browse and use skills"),
        ("status", "Display session configuration and token usage"),
        ("statusline", "Configure TUI status-line fields"),
        ("stop", "Stop all background terminals"),
        ("subagents", "Switch the active agent thread"),
        ("theme", "Choose a syntax-highlighting theme"),
        ("title", "Configure terminal window/tab title fields"),
        ("usage", "View account token usage"),
        ("vim", "Toggle Vim mode for the composer"),
    ]
    .into_iter()
    .map(|(name, description)| builtin(name, description, "codex"))
    .collect()
}

fn opencode_builtins() -> Vec<SlashCommand> {
    [
        ("clear", "Start a fresh OpenCode session"),
        ("compact", "Compact the current session context"),
        ("help", "Open OpenCode help"),
        ("models", "Choose an OpenCode provider and model"),
        ("new", "Start a new OpenCode session"),
        ("redo", "Restore the last reverted message"),
        ("sessions", "Browse OpenCode sessions"),
        ("share", "Share the current OpenCode session"),
        ("stats", "Show OpenCode token and cost statistics"),
        ("undo", "Revert the last OpenCode message"),
    ]
    .into_iter()
    .map(|(name, description)| builtin(name, description, "opencode"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalogs_include_native_commands() {
        let available = HashSet::from(["claude".into(), "codex".into()]);
        let list = list_commands(&available);
        assert!(list
            .iter()
            .any(|c| c.harness == "claude" && c.name == "compact"));
        assert!(list
            .iter()
            .any(|c| c.harness == "codex" && c.name == "status"));
        assert!(list.iter().any(|c| c.kind == "builtin"));
    }

    #[test]
    fn provider_memory_commands_come_from_the_catalog_not_a_name_table() {
        let all = HashSet::from(["claude".into(), "codex".into(), "opencode".into()]);
        let commands = provider_memory_commands(&all);
        assert!(commands
            .iter()
            .any(|c| c.harness == "claude" && c.name == "memory"));
        assert!(commands
            .iter()
            .any(|c| c.harness == "codex" && c.name == "memories"));
        assert!(
            commands.iter().all(|c| c.harness != "bridge"),
            "bridge-local pins are the ledger, not a provider memory"
        );
        assert!(
            provider_memory_commands(&HashSet::new()).is_empty(),
            "an unavailable adapter contributes nothing"
        );
    }

    #[test]
    fn dispatch_usage_and_clear() {
        let available = HashSet::from(["claude".into()]);
        assert!(matches!(
            dispatch("/usage", "claude", &available),
            SlashDispatch::Usage
        ));
        assert!(matches!(
            dispatch("/clear", "claude", &available),
            SlashDispatch::Clear
        ));
        assert!(matches!(
            dispatch("/voice", "codex", &available),
            SlashDispatch::Voice
        ));
        // The focus is carried, not merely present: it is forwarded to a
        // harness that accepts one and reported as ignored by one that does
        // not, so losing the text here would silently lose the instruction.
        assert!(matches!(
            dispatch("/compact focus on errors", "claude", &available),
            SlashDispatch::Compact { focus: Some(focus) } if focus == "focus on errors"
        ));
        assert!(matches!(
            dispatch("/compact", "claude", &available),
            SlashDispatch::Compact { focus: None }
        ));
        assert!(matches!(
            dispatch("/recall what did we decide", "claude", &available),
            SlashDispatch::Recall { query } if query == "what did we decide"
        ));
        assert!(matches!(
            dispatch("/pin I prefer Conventional Commits", "claude", &available),
            SlashDispatch::Pin { body } if body == "I prefer Conventional Commits"
        ));
        assert!(matches!(
            dispatch("/pins", "codex", &available),
            SlashDispatch::Pins
        ));
        assert!(matches!(
            dispatch("/unpin abcdef12", "claude", &available),
            SlashDispatch::Unpin { selector } if selector == "abcdef12"
        ));
        assert!(is_bridge_local("pin") && is_bridge_local("pins") && is_bridge_local("unpin"));
        let catalog = list_commands(&HashSet::new());
        assert!(catalog
            .iter()
            .any(|command| command.name == "recall" && command.harness == "bridge"));
        assert!(catalog
            .iter()
            .any(|command| command.name == "pin" && command.harness == "bridge"));
    }

    #[test]
    fn side_chat_commands_are_bridge_owned_on_every_harness() {
        let available = HashSet::from(["claude".into(), "codex".into(), "opencode".into()]);
        for harness in ["claude", "codex", "opencode"] {
            assert!(
                matches!(
                    dispatch("/btw is the plan sound?", harness, &available),
                    SlashDispatch::SideChat { command, query }
                        if command == "btw" && query == "is the plan sound?"
                ),
                "/btw must open a side chat, not forward to {harness}"
            );
            assert!(matches!(
                dispatch("/SIDE what did we pick?", harness, &available),
                SlashDispatch::SideChat { command, .. } if command == "side"
            ));
            assert!(is_bridge_local("btw") && is_bridge_local("side"));
        }
        // Bare command: still a side-chat request, with an empty question the
        // caller turns into usage guidance.
        assert!(matches!(
            dispatch("/btw", "claude", &available),
            SlashDispatch::SideChat { query, .. } if query.is_empty()
        ));
    }

    #[test]
    fn project_skills_are_listed_and_expanded_from_the_same_root() {
        let project = tempfile::tempdir().unwrap();
        let skill = project.path().join(".agents/skills/project-review");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: Review this project's invariants\n---\n\nUse the project contract.\n",
        )
        .unwrap();
        let available = HashSet::from(["codex".into()]);

        let catalog = list_commands_for_project(&available, Some(project.path()));
        assert!(catalog.iter().any(|command| {
            command.name == "project-review"
                && command.harness == "codex"
                && command.kind == "skill"
                && command.description == "Review this project's invariants"
        }));
        assert!(matches!(
            dispatch_for_project(
                "/project-review focus on auth",
                "codex",
                &available,
                Some(project.path()),
            ),
            SlashDispatch::Expand { text }
                if text.contains("Use the project contract.") && text.contains("focus on auth")
        ));
    }

    #[test]
    fn side_chat_commands_cannot_be_forwarded_as_provider_builtins() {
        let available = HashSet::from(["claude".into(), "codex".into(), "opencode".into()]);
        let catalog = list_commands(&available);
        for name in ["btw", "side"] {
            let entries: Vec<_> = catalog.iter().filter(|c| c.name == name).collect();
            assert_eq!(entries.len(), 1, "{name} must appear exactly once in the catalog");
            assert_eq!(entries[0].harness, "bridge", "{name} is Bridge-owned, not a provider builtin");
            assert!(!is_forwardable_builtin("claude", name));
            assert!(!is_forwardable_builtin("codex", name));
        }
    }
}
