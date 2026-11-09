//! A draft's Markdown as its destination takes it.
//!
//! Slack's connector takes standard Markdown and converts it itself, so a
//! Slack channel, thread or DM gets the body as written. Text pasted into
//! Slack by hand is different: Slack speaks mrkdwn (`*bold*`, `_italic_`, `<url|text>`, no headings),
//! GitHub speaks Markdown, and a plain destination gets the words with the
//! markup taken out. Code spans and fences pass through untouched.

use std::sync::OnceLock;

use bridge_protocol::messages::OutgoingFormat;
use regex::Regex;

use super::lint::{fenced_lines, map_prose};

/// The fence character of a fence line, if `line` is one.
fn fence_marker(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    (trimmed.chars().take_while(|c| *c == marker).count() >= 3).then(|| marker.to_string())
}

struct Patterns {
    bold: Regex,
    italic_star: Regex,
    italic_underscore: Regex,
    strike: Regex,
    link: Regex,
    heading: Regex,
    bullet: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        bold: Regex::new(r"\*\*([^*\n]+?)\*\*|__([^_\n]+?)__").unwrap(),
        italic_star: Regex::new(r"\*([^*\s][^*\n]*?)\*").unwrap(),
        // Only a whole word in underscores: `snake_case` and `my_file_v2`
        // keep theirs, as Markdown keeps them.
        italic_underscore: Regex::new(r"(^|[^\w])_([^_\s](?:[^_\n]*[^_\s])?)_([^\w]|$)").unwrap(),
        strike: Regex::new(r"~~([^~\n]+?)~~").unwrap(),
        link: Regex::new(r"\[([^\]\n]+)\]\(([^)\s]+)\)").unwrap(),
        heading: Regex::new(r"^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$").unwrap(),
        bullet: Regex::new(r"^(\s*)[*+-]\s+").unwrap(),
    })
}

/// Marks a converted bold run while single-star italics are rewritten.
const BOLD: char = '\u{1}';

fn slack_prose(text: &str) -> String {
    let p = patterns();
    let text = p.link.replace_all(text, "<$2|$1>");
    let text = p.bold.replace_all(&text, |caps: &regex::Captures| {
        let inner = caps.get(1).or_else(|| caps.get(2)).map_or("", |m| m.as_str());
        format!("{BOLD}{inner}{BOLD}")
    });
    let text = p.italic_star.replace_all(&text, "_${1}_");
    let text = p.strike.replace_all(&text, "~$1~");
    text.replace(BOLD, "*")
}

fn plain_prose(text: &str) -> String {
    let p = patterns();
    let text = p.link.replace_all(text, "$1 ($2)");
    let text = p.bold.replace_all(&text, |caps: &regex::Captures| {
        caps.get(1).or_else(|| caps.get(2)).map_or("", |m| m.as_str()).to_owned()
    });
    let text = p.italic_star.replace_all(&text, "$1");
    let text = p.italic_underscore.replace_all(&text, "$1$2$3");
    p.strike.replace_all(&text, "$1").into_owned()
}

/// `body` as `format` shows it.
pub fn render(body: &str, format: OutgoingFormat) -> String {
    if matches!(format, OutgoingFormat::Github | OutgoingFormat::SlackMarkdown) {
        return body.to_owned();
    }
    let p = patterns();
    let mut out = Vec::new();
    for (line, code, fence) in fenced_lines(body) {
        if fence {
            // Slack fences take no language tag; plain text drops fences.
            if format == OutgoingFormat::Slack {
                out.push(fence_marker(line).unwrap_or_else(|| "`".into()).repeat(3));
            }
            continue;
        }
        if code {
            out.push(line.to_owned());
            continue;
        }
        let rendered = if let Some(caps) = p.heading.captures(line) {
            let title = map_prose(&caps[1], plain_prose);
            if format == OutgoingFormat::Slack { format!("*{title}*") } else { title }
        } else {
            let bulleted = match format {
                OutgoingFormat::Slack => p.bullet.replace(line, "${1}\u{2022} ").into_owned(),
                _ => line.to_owned(),
            };
            match format {
                OutgoingFormat::Slack => map_prose(&bulleted, slack_prose),
                _ => map_prose(&bulleted, plain_prose).replace('`', ""),
            }
        };
        out.push(rendered);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_converts_emphasis_links_headings_and_bullets() {
        let body = "# Deploy **done**\n- **All** green, *mostly* ~~red~~\n* see [the run](https://ci.example/run_1?a=b)\n+ __third__";
        assert_eq!(
            render(body, OutgoingFormat::Slack),
            "*Deploy done*\n\u{2022} *All* green, _mostly_ ~red~\n\u{2022} see <https://ci.example/run_1?a=b|the run>\n\u{2022} *third*"
        );
    }

    #[test]
    fn slack_leaves_code_untouched() {
        let body = "Run `**not bold**` then\n```rust\nlet x = *y; // [a](b)\n```";
        assert_eq!(render(body, OutgoingFormat::Slack), "Run `**not bold**` then\n```\nlet x = *y; // [a](b)\n```");
    }

    #[test]
    fn longer_fences_and_double_backtick_spans_render_as_code() {
        let body = "````\n```\n**kept**\n```\n````\nsee ``**raw**`` and **bold**";
        assert_eq!(render(body, OutgoingFormat::Slack), "```\n```\n**kept**\n```\n```\nsee ``**raw**`` and *bold*");
    }

    #[test]
    fn plain_keeps_a_fence_example_inside_a_longer_fence() {
        assert_eq!(render("````\n```\nexample\n```\n````", OutgoingFormat::Plain), "```\nexample\n```");
    }

    #[test]
    fn plain_strips_markup_and_keeps_link_targets() {
        let body = "## Note\n**Done**, see [run](https://ci/1) and `cargo test`\n```\nraw *text*\n```";
        assert_eq!(render(body, OutgoingFormat::Plain), "Note\nDone, see run (https://ci/1) and cargo test\nraw *text*");
    }

    #[test]
    fn slack_through_the_connector_is_the_body_as_written() {
        let body = "# Title\n| a |\n|---|\n```python\nprint(1)\n```";
        assert_eq!(render(body, OutgoingFormat::SlackMarkdown), body);
    }

    #[test]
    fn plain_strips_underscore_emphasis_but_not_identifiers() {
        assert_eq!(render("This is _important_, really", OutgoingFormat::Plain), "This is important, really");
        assert_eq!(render("_lead_ in", OutgoingFormat::Plain), "lead in");
        assert_eq!(render("set snake_case_name and __init__ here", OutgoingFormat::Plain), "set snake_case_name and init here");
        assert_eq!(render("path my_file_v2.txt stays", OutgoingFormat::Plain), "path my_file_v2.txt stays");
    }

    #[test]
    fn github_is_the_body_as_written() {
        let body = "# Title\n| a |\n|---|";
        assert_eq!(render(body, OutgoingFormat::Github), body);
    }
}
