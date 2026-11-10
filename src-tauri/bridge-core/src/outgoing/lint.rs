//! The built-in style rules a draft is checked against.
//!
//! Pure functions of the text and the destination's format. Two kinds of
//! rule: hard rules Bridge can fix by itself (em dashes, AI attribution
//! lines) and format warnings it can only point at (a table Slack will show
//! as raw pipes). Text inside code spans and fences is never touched: an em
//! dash in a log line is data, not style.

use std::sync::OnceLock;

use bridge_protocol::messages::{OutgoingFormat, OutgoingLintFinding, SLACK_MAX_ELEMENT_CHARS};
use regex::Regex;

pub const NO_EM_DASH: &str = "no_em_dash";
pub const NO_AI_ATTRIBUTION: &str = "no_ai_attribution";
pub const SLACK_TABLE: &str = "slack_table";
pub const SLACK_HEADING: &str = "slack_heading";
pub const MAX_LINES: &str = "max_lines";
pub const SLACK_LENGTH: &str = "slack_length";

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

/// The opening run of a fence line: its character and length, if the line
/// opens or could close a fence (up to three spaces of indent, then three or
/// more backticks or tildes).
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let marker = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = rest.chars().take_while(|c| *c == marker).count();
    (length >= 3).then(|| (marker, length, &rest[length..]))
}

/// Each line with whether it is code (inside a fenced block or one of its
/// fence lines) and whether it is itself a fence line. A fence closes only
/// on the same character, at least as many of them, and nothing else on the
/// line, so a four-backtick fence can hold a three-backtick example.
pub(crate) fn fenced_lines(body: &str) -> Vec<(&str, bool, bool)> {
    let mut open: Option<(char, usize)> = None;
    body.split('\n')
        .map(|line| match (open, fence_run(line)) {
            (None, Some((marker, length, info))) if marker == '~' || !info.contains('`') => {
                open = Some((marker, length));
                (line, true, true)
            }
            (Some((marker, length)), Some((closing, run, rest))) if closing == marker && run >= length && rest.trim().is_empty() => {
                open = None;
                (line, true, true)
            }
            (Some(_), _) => (line, true, false),
            (None, _) => (line, false, false),
        })
        .collect()
}

fn lines_with_fences(body: &str) -> Vec<(&str, bool)> {
    fenced_lines(body).into_iter().map(|(line, code, _)| (line, code)).collect()
}

/// A line split into prose and inline code. A run of backticks opens a
/// code span only if a run of exactly the same length closes it later on
/// the line; an unmatched run is just text.
fn segments(line: &str) -> Vec<(bool, &str)> {
    let bytes = line.as_bytes();
    let run_at = |at: usize| bytes[at..].iter().take_while(|byte| **byte == b'`').count();
    let mut out = Vec::new();
    let (mut prose_start, mut at) = (0, 0);
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let length = run_at(at);
        let mut search = at + length;
        let mut close = None;
        while search < bytes.len() {
            if bytes[search] == b'`' {
                let run = run_at(search);
                if run == length {
                    close = Some(search);
                    break;
                }
                search += run;
            } else {
                search += 1;
            }
        }
        match close {
            Some(close) => {
                if prose_start < at {
                    out.push((false, &line[prose_start..at]));
                }
                out.push((true, &line[at..close + length]));
                at = close + length;
                prose_start = at;
            }
            None => at += length,
        }
    }
    if prose_start < line.len() {
        out.push((false, &line[prose_start..]));
    }
    out
}

/// Apply `transform` to the prose of one line, leaving inline code alone.
pub(crate) fn map_prose(line: &str, transform: impl Fn(&str) -> String) -> String {
    segments(line).into_iter().map(|(code, part)| if code { part.to_owned() } else { transform(part) }).collect()
}

fn prose_has(line: &str, needle: char) -> bool {
    segments(line).into_iter().any(|(code, part)| !code && part.contains(needle))
}

/// The line with its inline code removed: what a structural check reads.
pub(crate) fn prose_only(line: &str) -> String {
    segments(line).into_iter().filter(|(code, _)| !code).map(|(_, part)| part).collect()
}

fn footer_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)^(?:\x{1F916}\s*)?generated (?:with|by) (?:\[claude code\]\([^)\s]*\)|claude code)\.?$").unwrap()
    })
}

/// A whole line that only attributes the text to an assistant: a
/// `Co-Authored-By:` trailer naming one, or the "Generated with Claude Code"
/// footer. Code spans are not part of the line for this check, so a sentence
/// that quotes the footer, or mentions it, stays.
fn attribution_line(line: &str) -> bool {
    let prose = prose_only(line);
    let text = prose.trim();
    if text != line.trim() {
        return false;
    }
    let lower = text.to_lowercase();
    let assistant = ["claude", "anthropic", "openai", "codex", "chatgpt", "copilot", "gpt-"];
    (lower.starts_with("co-authored-by:") && assistant.iter().any(|name| lower.contains(name))) || footer_pattern().is_match(text)
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
    // Text pasted into Slack by hand stays raw: tables and headings do not
    // survive. Through the connector they do, so only the clipboard warns.
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
    }
    if format == OutgoingFormat::SlackMarkdown {
        let count = body.chars().count();
        if count > SLACK_MAX_ELEMENT_CHARS {
            findings.push(finding(
                SLACK_LENGTH,
                format!("{count} characters: Slack takes at most {SLACK_MAX_ELEMENT_CHARS} in one message"),
                None,
                false,
                false,
            ));
        }
    }
    if matches!(format, OutgoingFormat::Slack | OutgoingFormat::SlackMarkdown) {
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
    fn multi_backtick_spans_and_longer_fences_are_code() {
        let span = "Run ``a \u{2014} b`` now";
        assert_eq!(fix(span).0, span, "a double-backtick span is code");
        let nested = "````md\n```\nlog \u{2014} line\n```\n````\nafter \u{2014} this";
        let (text, fixed) = fix(nested);
        assert_eq!(text, "````md\n```\nlog \u{2014} line\n```\n````\nafter, this", "only the prose after the outer fence changes");
        assert_eq!(fixed.len(), 1);
        // A lone backtick opens nothing, so the dash after it is prose.
        assert_eq!(fix("it's a ` tick \u{2014} really").0, "it's a ` tick, really");
        // A fence closes only on its own character.
        let tilde = "~~~\n```\nx \u{2014} y\n~~~";
        assert_eq!(fix(tilde).0, tilde);
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
    fn sentences_and_code_that_mention_the_footer_are_left_alone() {
        for body in [
            "Use `Generated with Claude Code` to find those files",
            "We generated with Claude Code a quick prototype yesterday.",
            "The commit trailer looks like Co-Authored-By: Claude, which we strip.",
        ] {
            let (text, fixed) = fix(body);
            assert_eq!(text, body);
            assert!(fixed.is_empty(), "{body}");
        }
        // The real footers are still whole lines and still go.
        for footer in ["Generated with Claude Code", "\u{1F916} Generated with [Claude Code](https://claude.com/claude-code)", "co-authored-by: Codex <codex@openai.com>"] {
            assert_eq!(fix(&format!("Done.\n\n{footer}")).0, "Done.", "{footer}");
        }
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
    fn slack_through_the_connector_keeps_tables_and_headings_but_not_length() {
        let body = "# Status\n| a | b |\n|---|---|\n| 1 | 2 |\n```rust\nfn main() {}\n```";
        assert!(check(body, OutgoingFormat::SlackMarkdown).is_empty());
        let long = "x".repeat(SLACK_MAX_ELEMENT_CHARS + 1);
        let findings = check(&long, OutgoingFormat::SlackMarkdown);
        assert_eq!(rules(&findings), vec![SLACK_LENGTH]);
        assert!(findings[0].message.starts_with("5001 characters"));
        assert!(check(&"x".repeat(SLACK_MAX_ELEMENT_CHARS), OutgoingFormat::SlackMarkdown).is_empty());
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
