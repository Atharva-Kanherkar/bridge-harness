//! The built-in style rules a draft is checked against.
//!
//! Pure functions of the text and the destination's format. Two kinds of
//! rule: hard rules Bridge can fix by itself (em dashes, AI attribution
//! lines) and format warnings it can only point at (a table Slack will show
//! as raw pipes). Text inside code spans and fences is never touched: an em
//! dash in a log line is data, not style.

use std::sync::OnceLock;

use bridge_protocol::messages::{OutgoingFormat, OutgoingLintFinding};
use regex::Regex;

pub const NO_EM_DASH: &str = "no_em_dash";
pub const NO_AI_ATTRIBUTION: &str = "no_ai_attribution";
pub const SLACK_TABLE: &str = "slack_table";
pub const SLACK_HEADING: &str = "slack_heading";
pub const MAX_LINES: &str = "max_lines";

/// Slack messages past this many lines get a warning.
pub const SLACK_MAX_LINES: usize = 15;

const EM_DASH: char = '\u{2014}';

fn finding(rule: &str, message: impl Into<String>, line: Option<usize>, fixable: bool, fixed: bool) -> OutgoingLintFinding {
    OutgoingLintFinding {
        rule: rule.to_owned(),
        message: message.into(),
        line: line.map(|line| line as u32),
        fixable,
        fixed,
    }
}

fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// Each line with whether it sits inside a fenced block (fence lines count as inside).
fn lines_with_fences(body: &str) -> Vec<(&str, bool)> {
    let mut inside = false;
    body.split('\n')
        .map(|line| {
            if is_fence(line) {
                inside = !inside;
                return (line, true);
            }
            (line, inside)
        })
        .collect()
}

/// Apply `transform` to the prose of one line, leaving `inline code` alone.
pub(crate) fn map_prose(line: &str, transform: impl Fn(&str) -> String) -> String {
    line.split('`')
        .enumerate()
        .map(|(index, part)| if index % 2 == 0 { transform(part) } else { part.to_owned() })
        .collect::<Vec<_>>()
        .join("`")
}

fn prose_has(line: &str, needle: char) -> bool {
    line.split('`').enumerate().any(|(index, part)| index % 2 == 0 && part.contains(needle))
}

fn attribution_line(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    let assistant = ["claude", "anthropic", "openai", "codex", "chatgpt", "copilot", "gpt-"];
    (lower.starts_with("co-authored-by:") && assistant.iter().any(|name| lower.contains(name)))
        || lower.contains("generated with [claude code]")
        || lower.contains("generated with claude code")
        || lower.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).starts_with("generated with [claude")
}

fn dash_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"[ \t]*\x{2014}[ \t]*").unwrap())
}

fn replace_dashes(prose: &str) -> String {
    let replaced = dash_pattern().replace_all(prose, ", ").into_owned();
    // A dash that opened or closed the text leaves a dangling comma.
    let replaced = replaced.strip_prefix(", ").map(str::to_owned).unwrap_or(replaced);
    replaced.strip_suffix(", ").map(str::to_owned).unwrap_or(replaced).replace(",,", ",")
}

fn is_table_divider(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('-')
        && trimmed.contains('|')
        && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ')
}

/// What is wrong with `body` for `format`, nothing changed. Every finding
/// comes back with `fixed: false`.
pub fn check(body: &str, format: OutgoingFormat) -> Vec<OutgoingLintFinding> {
    let lines = lines_with_fences(body);
    let mut findings = Vec::new();
    for (index, (line, in_fence)) in lines.iter().enumerate() {
        let number = index + 1;
        if !in_fence && prose_has(line, EM_DASH) {
            findings.push(finding(NO_EM_DASH, "Em dash", Some(number), true, false));
        }
        if !in_fence && attribution_line(line) {
            findings.push(finding(NO_AI_ATTRIBUTION, "AI attribution line", Some(number), true, false));
        }
    }
    if format == OutgoingFormat::Slack {
        for (index, (line, in_fence)) in lines.iter().enumerate() {
            if *in_fence {
                continue;
            }
            let next = lines.get(index + 1).map(|(next, _)| *next).unwrap_or_default();
            if line.trim_start().starts_with('|') && is_table_divider(next) {
                findings.push(finding(SLACK_TABLE, "Table: Slack shows it as raw text", Some(index + 1), false, false));
            }
            if is_heading(line) {
                findings.push(finding(SLACK_HEADING, "Heading: Slack has none, it will show in bold", Some(index + 1), false, false));
            }
        }
        let count = body.trim_end().split('\n').count();
        if count > SLACK_MAX_LINES {
            findings.push(finding(
                MAX_LINES,
                format!("{count} lines: Slack messages read best under {SLACK_MAX_LINES}"),
                None,
                false,
                false,
            ));
        }
    }
    findings
}

/// Apply every fixable rule. Returns the new text and one `fixed: true`
/// finding per change, so the card can say what Bridge did.
pub fn fix(body: &str) -> (String, Vec<OutgoingLintFinding>) {
    let mut fixed = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    for (index, (line, in_fence)) in lines_with_fences(body).into_iter().enumerate() {
        let number = index + 1;
        if !in_fence && attribution_line(line) {
            fixed.push(finding(NO_AI_ATTRIBUTION, "AI attribution line removed", Some(number), true, true));
            continue;
        }
        if !in_fence && prose_has(line, EM_DASH) {
            fixed.push(finding(NO_EM_DASH, "Em dash replaced with a comma", Some(number), true, true));
            kept.push(map_prose(line, replace_dashes));
            continue;
        }
        kept.push(line.to_owned());
    }
    let mut text = kept.join("\n");
    if fixed.iter().any(|finding| finding.rule == NO_AI_ATTRIBUTION) {
        // A removed footer leaves the blank line that introduced it.
        text = text.trim_end().to_owned();
    }
    (text, fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(findings: &[OutgoingLintFinding]) -> Vec<&str> {
        findings.iter().map(|finding| finding.rule.as_str()).collect()
    }

    #[test]
    fn em_dash_spaced_and_tight_become_commas() {
        let (text, fixed) = fix("Deploy is done \u{2014} all green\nShip it\u{2014}today\n\u{2014} leading");
        assert_eq!(text, "Deploy is done, all green\nShip it, today\nleading");
        assert_eq!(rules(&fixed), vec![NO_EM_DASH, NO_EM_DASH, NO_EM_DASH]);
        assert!(fixed.iter().all(|finding| finding.fixed && finding.fixable));
        assert_eq!(fixed[1].line, Some(2));
    }

    #[test]
    fn em_dash_inside_code_is_left_alone() {
        let body = "Run `a \u{2014} b` now\n```\nlog \u{2014} line\n```";
        assert!(check(body, OutgoingFormat::Github).is_empty());
        let (text, fixed) = fix(body);
        assert_eq!(text, body);
        assert!(fixed.is_empty());
    }

    #[test]
    fn ai_attribution_lines_are_removed() {
        let body = "Fixed the flake.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\u{1F916} Generated with [Claude Code](https://claude.com/claude-code)";
        assert_eq!(rules(&check(body, OutgoingFormat::Github)), vec![NO_AI_ATTRIBUTION, NO_AI_ATTRIBUTION]);
        let (text, fixed) = fix(body);
        assert_eq!(text, "Fixed the flake.");
        assert_eq!(fixed.len(), 2);
        // A human co-author is not attribution to an assistant.
        assert!(check("Co-Authored-By: Nina <nina@example.com>", OutgoingFormat::Plain).is_empty());
    }

    #[test]
    fn slack_flags_tables_headings_and_length() {
        let body = "# Status\n| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(rules(&check(body, OutgoingFormat::Slack)), vec![SLACK_HEADING, SLACK_TABLE]);
        let long = (1..=16).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let findings = check(&long, OutgoingFormat::Slack);
        assert_eq!(rules(&findings), vec![MAX_LINES]);
        assert!(findings[0].message.starts_with("16 lines"));
        assert!(!findings[0].fixable);
    }

    #[test]
    fn github_and_plain_skip_slack_rules() {
        let body = "# Status\n| a | b |\n|---|---|";
        assert!(check(body, OutgoingFormat::Github).is_empty());
        assert!(check(body, OutgoingFormat::Plain).is_empty());
    }

    #[test]
    fn fixing_is_idempotent() {
        let body = "One \u{2014} two\n\nCo-Authored-By: Codex <codex@openai.com>";
        let (once, _) = fix(body);
        let (twice, again) = fix(&once);
        assert_eq!(once, twice);
        assert!(again.is_empty());
        assert!(check(&once, OutgoingFormat::Slack).iter().all(|finding| !finding.fixable));
    }
}
