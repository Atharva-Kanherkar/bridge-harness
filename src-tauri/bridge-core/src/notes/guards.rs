//! Server-side guards shared by the model passes. A model's reply is a
//! suggestion; these decide what of it may be shown or written.

use serde_json::Value;

use super::jobs::ProjectChoice;
use super::service::KINDS;

/// The first JSON object in a reply, tolerating prose or a code fence around it.
pub fn json_object(text: &str) -> Option<serde_json::Map<String, Value>> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    match serde_json::from_str::<Value>(&text[start..=end]).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

pub fn string(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned()
}

/// Collapse whitespace runs to one space and trim.
pub fn normalize_space(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `excerpt` appears in `body`, ignoring differences in whitespace.
/// This is the grounding check: an item or task that cannot quote the page
/// is something the model made up.
pub fn quoted_in(excerpt: &str, body: &str) -> bool {
    let excerpt = normalize_space(excerpt);
    !excerpt.is_empty() && normalize_space(body).contains(&excerpt)
}

/// At most `max` characters, cut on a character boundary.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// A single-line title of 1..=`max` characters, or `None`.
pub fn title(raw: &str, max: usize) -> Option<String> {
    let line = raw.trim().trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '*' | '#')).trim();
    if line.is_empty() || line.contains('\n') || line.chars().count() > max {
        return None;
    }
    Some(line.to_owned())
}

pub fn kind(raw: &str) -> String {
    let lowered = raw.trim().to_lowercase();
    if KINDS.contains(&lowered.as_str()) {
        lowered
    } else {
        "note".to_owned()
    }
}

/// Lowercase `[a-z0-9-]{1,24}` tags, deduplicated, at most `max`.
pub fn tags(value: Option<&Value>, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in value.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let mut tag = String::new();
        for character in raw.trim().trim_start_matches('#').chars().flat_map(char::to_lowercase) {
            if character.is_ascii_alphanumeric() {
                tag.push(character);
            } else if (character == '-' || character == ' ' || character == '_') && !tag.ends_with('-') && !tag.is_empty() {
                tag.push('-');
            }
        }
        let tag = tag.trim_end_matches('-').to_owned();
        if !tag.is_empty() && tag.len() <= 24 && !out.contains(&tag) {
            out.push(tag);
        }
        if out.len() == max {
            break;
        }
    }
    out
}

/// A project key (`p1`..) back to a workspace id, if it was listed.
pub fn project(value: Option<&Value>, projects: &[ProjectChoice]) -> Option<String> {
    let key = value.and_then(Value::as_str)?.trim().to_lowercase();
    let index: usize = key.strip_prefix('p')?.parse().ok()?;
    projects.get(index.checked_sub(1)?).map(|project| project.id.clone())
}

/// The projects block of a prompt.
pub fn projects_block(projects: &[ProjectChoice]) -> String {
    let mut block = String::from("<projects>\n");
    for (index, project) in projects.iter().take(30).enumerate() {
        block.push_str(&format!("p{}: {}\n", index + 1, project.label));
    }
    block.push_str("</projects>\n");
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quotes_ignore_whitespace_but_not_words() {
        let body = "Ideas:\n  - build a   notes app\n- call mom";
        assert!(quoted_in("build a notes app", body));
        assert!(quoted_in("- build a notes\napp", body));
        assert!(!quoted_in("build a todo app", body));
        assert!(!quoted_in("   ", body));
    }

    #[test]
    fn tags_projects_and_titles_are_normalized() {
        assert_eq!(tags(Some(&json!(["#Launch", "launch", "Side Project", "q4!", "a-really-long-tag-that-goes-on-forever", "x", "y"])), 4), vec!["launch", "side-project", "q4", "x"]);
        let projects = vec![ProjectChoice { id: "ws-a".into(), label: "A".into() }, ProjectChoice { id: "ws-b".into(), label: "B".into() }];
        assert_eq!(project(Some(&json!("p2")), &projects).as_deref(), Some("ws-b"));
        assert_eq!(project(Some(&json!("p3")), &projects), None);
        assert_eq!(project(Some(&json!("ws-a")), &projects), None);
        assert_eq!(project(Some(&json!(null)), &projects), None);
        assert_eq!(title("  \"Launch plan\" ", 80).as_deref(), Some("Launch plan"));
        assert_eq!(title("two\nlines", 80), None);
        assert_eq!(title(&"x".repeat(81), 80), None);
        assert_eq!(kind("IDEA"), "idea");
        assert_eq!(kind("essay"), "note");
        assert_eq!(clip("abcdef", 4), "abc…");
    }
}
