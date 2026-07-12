//! Delegation protocol for Bridge's multi-agent tree.
//!
//! A running session (the orchestrator, or any worker) delegates work by
//! emitting a fenced directive block in its assistant message — a code fence
//! tagged "bridge-delegate" whose body is a JSON object with harness, model,
//! effort, task, and optional context.
//!
//! Bridge parses completed assistant messages, spawns the requested child
//! session in the SAME workspace/worktree (shared context), sends it the task,
//! and forwards the child's final summary back to the parent as a new turn.
//! Children may delegate again up to [`MAX_DEPTH`], which is how the tree in the
//! blueprint (orchestrator -> claude/codex -> codex/claude) is formed.
//!
//! No API keys and no MCP server are involved: every child is a local codex or
//! claude process launched through the existing structured adapters using the
//! subscription credentials already on the Mac.

use serde_json::Value;

/// Deepest level a session may occupy. Depth 0 is the orchestrator; workers are
/// depth >= 1. A session at `MAX_DEPTH` is told to finish the work itself
/// instead of delegating further, which bounds the tree.
pub const MAX_DEPTH: i64 = 3;

/// Most children a single assistant message may spawn, so one confused turn
/// cannot fan out unbounded worker processes.
pub const MAX_FANOUT: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    pub task: String,
    pub context: Option<String>,
}

impl Directive {
    fn from_value(value: &Value) -> Option<Self> {
        let harness = normalize_harness(value.get("harness").and_then(Value::as_str)?)?;
        let task = value
            .get("task")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|task| !task.is_empty())?
            .to_owned();
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .map(|model| normalize_model(&harness, model))
            .unwrap_or_else(|| default_model(&harness).to_owned());
        let effort = value
            .get("effort")
            .and_then(Value::as_str)
            .map(normalize_effort);
        let context = value
            .get("context")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|context| !context.is_empty())
            .map(str::to_owned);
        Some(Self {
            harness,
            model,
            effort,
            task,
            context,
        })
    }

    /// Short human label for the worker session, e.g. `Claude · Fable`.
    pub fn label(&self) -> String {
        format!(
            "{} · {}",
            match self.harness.as_str() {
                "claude" => "Claude",
                _ => "Codex",
            },
            model_display(&self.model)
        )
    }
}

pub fn normalize_harness(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" | "anthropic" => Some("claude".into()),
        "codex" | "gpt" | "openai" => Some("codex".into()),
        _ => None,
    }
}

fn default_model(harness: &str) -> &'static str {
    match harness {
        "claude" => "sonnet",
        _ => "gpt-5.6-luna",
    }
}

fn normalize_model(harness: &str, model: &str) -> String {
    let value = model.trim().to_ascii_lowercase();
    if harness == "claude" {
        return match value.as_str() {
            "sonnet" | "claude-sonnet" => "sonnet",
            "opus" | "claude-opus" => "opus",
            "haiku" | "claude-haiku" => "haiku",
            "fable" | "claude-fable" => "fable",
            _ => "sonnet",
        }
        .to_owned();
    }
    match value.as_str() {
        "luna" | "gpt-luna" | "gpt-5.6-luna" => "gpt-5.6-luna",
        "terra" | "gpt-terra" | "gpt-5.6-terra" => "gpt-5.6-terra",
        "sol" | "gpt-sol" | "gpt-5.6-sol" => "gpt-5.6-sol",
        "codex" | "gpt-5.3-codex" => "gpt-5.3-codex",
        _ => "gpt-5.6-luna",
    }
    .to_owned()
}

pub fn model_display(model: &str) -> String {
    match model {
        "sonnet" => "Sonnet",
        "opus" => "Opus",
        "haiku" => "Haiku",
        "fable" => "Fable",
        "gpt-5.6-luna" => "GPT Luna",
        "gpt-5.6-terra" => "GPT Terra",
        "gpt-5.6-sol" => "GPT Sol",
        "gpt-5.3-codex" => "GPT-5.3 Codex",
        other => other,
    }
    .to_owned()
}

/// Collapse free-form effort words into the four tiers Bridge routes on.
pub fn normalize_effort(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "min" | "minimal" | "light" => "low",
        "high" => "high",
        "xhigh" | "x-high" | "extra" | "very-high" | "very high" | "ultra" | "max" | "maximum" => {
            "xhigh"
        }
        _ => "medium",
    }
    .to_owned()
}

/// Parse every delegation directive found in an assistant message.
pub fn parse_directives(text: &str) -> Vec<Directive> {
    let mut directives = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") {
            continue;
        }
        let tag = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
        if !(tag.contains("bridge") && tag.contains("delegate")) {
            continue;
        }
        let mut body = String::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            body.push_str(inner);
            body.push('\n');
        }
        let Ok(value) = serde_json::from_str::<Value>(body.trim()) else {
            continue;
        };
        match value {
            Value::Array(items) => {
                for item in &items {
                    if let Some(directive) = Directive::from_value(item) {
                        directives.push(directive);
                    }
                }
            }
            _ => {
                if let Some(directive) = Directive::from_value(&value) {
                    directives.push(directive);
                }
            }
        }
    }
    directives
}

/// Remove delegation blocks from a message so the conversation surface shows the
/// agent's prose, not the machine directive (the delegation card carries that).
pub fn strip_directives(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            let tag = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
            if tag.contains("bridge") && tag.contains("delegate") {
                for inner in lines.by_ref() {
                    if inner.trim_start().starts_with("```") {
                        break;
                    }
                }
                continue;
            }
        }
        kept.push(line);
    }
    kept.join("\n").trim().to_owned()
}

/// The shared delegation protocol injected into every session. `depth` is the
/// session's own level; when it has reached [`MAX_DEPTH`] the protocol tells it
/// to stop delegating and finish the work directly.
pub fn protocol(depth: i64) -> String {
    let budget = if depth >= MAX_DEPTH {
        format!(
            "You are at the maximum delegation depth ({MAX_DEPTH}). Do NOT delegate further — complete this work yourself."
        )
    } else {
        format!(
            "You may delegate to worker agents. You are at depth {depth}; workers you spawn run at depth {}. The tree is capped at depth {MAX_DEPTH}.",
            depth + 1
        )
    };
    format!(
        r#"## Delegating work (Bridge multi-agent protocol)

Bridge lets you hand a subtask to another coding agent — Claude Code or Codex — on a specific model and reasoning effort. {budget}

To delegate, emit a fenced block EXACTLY like this in your reply (Bridge intercepts it; the user does not have to do anything):

```bridge-delegate
{{"harness": "claude", "model": "fable", "effort": "high", "task": "Precise, self-contained instructions for the worker", "context": "Any background the worker needs"}}
```

Rules:
- `harness`: "claude" or "codex".
- `model`: claude → sonnet | opus | haiku | fable. codex → gpt-5.6-luna | gpt-5.6-terra | gpt-5.6-sol | gpt-5.3-codex.
- `effort`: low | medium | high | xhigh. Match effort to difficulty; cheap+low for trivial work, strong model + high/xhigh for heavy or high-stakes work.
- `task`: everything the worker needs; it does not see this conversation, only your task + context.
- Workers share this workspace and its files. Prefer delegating ONE coding worker at a time and waiting for its result before the next, to avoid conflicting edits. Multiple read-only/analysis workers in one message are fine.
- After you delegate, STOP and wait. Bridge runs the worker and replies to you with a `[worker result]` message. Then continue, delegate again, or give your final answer.
- If the task is small, just do it yourself instead of delegating."#
    )
}

/// System instructions handed to a freshly spawned worker.
pub fn worker_briefing(directive: &Directive, depth: i64, branch: &str) -> String {
    let effort = directive.effort.as_deref().unwrap_or("medium");
    let context = directive
        .context
        .as_deref()
        .map(|context| format!("\n\n## Context from your parent\n{context}"))
        .unwrap_or_default();
    let effort_hint = match effort {
        "low" => "Work quickly and directly; keep reasoning minimal.",
        "high" => "Think carefully and reason thoroughly before acting.",
        "xhigh" => "This is heavy, high-stakes work. Reason exhaustively; verify your work before finishing.",
        _ => "Balance speed and rigor.",
    };
    format!(
        r#"You are a Bridge worker agent spawned by an orchestrator to complete one focused task.

You are running in the shared workspace on branch `{branch}` at reasoning effort `{effort}`. {effort_hint}

Do the task in the first user message. When done, end your turn with a concise summary of what you changed or found — that summary is sent back to your parent, so make it self-contained. Do not ask the parent questions unless truly blocked.{context}

{protocol}"#,
        protocol = protocol(depth)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_directive() {
        let text = "I'll delegate this.\n\n```bridge-delegate\n{\"harness\":\"claude\",\"model\":\"fable\",\"effort\":\"high\",\"task\":\"Refactor auth\"}\n```\n";
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].harness, "claude");
        assert_eq!(directives[0].model, "fable");
        assert_eq!(directives[0].effort.as_deref(), Some("high"));
        assert_eq!(directives[0].task, "Refactor auth");
    }

    #[test]
    fn parses_colon_tag_and_array() {
        let text = "```bridge:delegate\n[{\"harness\":\"codex\",\"task\":\"a\"},{\"harness\":\"claude\",\"task\":\"b\"}]\n```";
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].harness, "codex");
        assert_eq!(directives[0].model, "gpt-5.6-luna");
        assert_eq!(directives[1].harness, "claude");
    }

    #[test]
    fn normalizes_aliases_and_effort() {
        let text = "```bridge-delegate\n{\"harness\":\"anthropic\",\"model\":\"opus\",\"effort\":\"ultra\",\"task\":\"x\"}\n```";
        let directive = &parse_directives(text)[0];
        assert_eq!(directive.harness, "claude");
        assert_eq!(directive.model, "opus");
        assert_eq!(directive.effort.as_deref(), Some("xhigh"));
    }

    #[test]
    fn ignores_non_bridge_fences_and_bad_json() {
        let text = "```python\nprint('hi')\n```\n```bridge-delegate\nnot json\n```";
        assert!(parse_directives(text).is_empty());
    }

    #[test]
    fn rejects_directive_without_task_or_bad_harness() {
        let missing = "```bridge-delegate\n{\"harness\":\"claude\"}\n```";
        assert!(parse_directives(missing).is_empty());
        let bad = "```bridge-delegate\n{\"harness\":\"gemini\",\"task\":\"x\"}\n```";
        assert!(parse_directives(bad).is_empty());
    }

    #[test]
    fn strips_directive_blocks_but_keeps_prose() {
        let text = "Handing this to a worker.\n\n```bridge-delegate\n{\"harness\":\"claude\",\"task\":\"x\"}\n```\n\nStanding by.";
        let stripped = strip_directives(text);
        assert!(stripped.contains("Handing this to a worker."));
        assert!(stripped.contains("Standing by."));
        assert!(!stripped.contains("bridge-delegate"));
        assert!(!stripped.contains("harness"));
    }

    #[test]
    fn protocol_forbids_delegation_at_max_depth() {
        assert!(protocol(MAX_DEPTH).contains("maximum delegation depth"));
        assert!(protocol(0).contains("You may delegate"));
    }

    #[test]
    fn worker_briefing_carries_task_effort_and_protocol() {
        let directive = Directive {
            harness: "codex".into(),
            model: "gpt-5.6-sol".into(),
            effort: Some("xhigh".into()),
            task: "Do the thing".into(),
            context: Some("background".into()),
        };
        let briefing = worker_briefing(&directive, 1, "bridge/task-kyoto");
        assert!(briefing.contains("bridge/task-kyoto"));
        assert!(briefing.contains("xhigh"));
        assert!(briefing.contains("background"));
        assert!(briefing.contains("bridge-delegate"));
    }
}
