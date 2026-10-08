//! `VisualSpec` v1 validation.
//!
//! Authoritative: the `visualize` tool refuses anything this rejects, and
//! names every problem by its JSON path so the model can fix the call in one
//! retry. The frontend parser (`src/mcp-apps/spec.ts`) mirrors these rules as
//! a second line of defence; both run `testing/fixtures/visualize/specs.json`
//! and must agree.
//!
//! The model controls data, never presentation it could abuse: no HTML, no
//! URLs to load, no colours outside the token enum, no Vega-Lite features this
//! build does not draw.

use serde::Serialize;
use serde_json::{Map, Value};

use super::catalog;

pub const MAX_SPEC_BYTES: usize = 256 * 1024;
pub const MAX_BLOCKS: usize = 6;
pub const MAX_ROWS: usize = 5000;
pub const MAX_SOURCES: usize = 30;
pub const MAX_FOLLOW_UPS: usize = 4;
pub const MAX_NOTES: usize = 6;
pub const MAX_DIAGRAM_NODES: usize = 300;
/// How many problems one refusal lists. Past this the model is better served
/// by fixing the first batch and calling again.
pub const MAX_REPORTED: usize = 20;

pub const COLORS: &[&str] = &[
    "claude", "codex", "opencode", "cursor", "neutral", "accent", "positive", "negative",
];

const SOURCE_KINDS: &[&str] = &["web", "tool", "file", "user", "computed", "estimate"];
const ENCODING_TYPES: &[&str] = &["quantitative", "nominal", "ordinal", "temporal"];
const AGGREGATES: &[&str] = &["sum", "mean", "average", "count", "min", "max", "median"];
const TOP_KEYS: &[&str] = &[
    "version", "title", "subtitle", "layout", "blocks", "sources", "followUps", "notes", "redraw",
];
const BLOCK_KEYS: &[&str] = &[
    "family", "form", "title", "vegaLite", "colors", "sourceIds", "content", "graph",
];
/// Vega-Lite features this build does not draw, with what to do instead.
const FORBIDDEN_VEGA: &[(&str, &str)] = &[
    ("url", "pass the rows inline in data.values"),
    ("config", "Bridge owns the theme; drop config"),
    ("usermeta", "drop usermeta"),
    ("href", "links are not supported in a visual"),
    ("params", "interaction is built in; drop params"),
    ("selection", "interaction is built in; drop selection"),
    ("signals", "drop signals"),
    ("transform", "compute derived values yourself and pass them in data.values"),
    ("datasets", "pass the rows inline in data.values"),
    ("layer", "use one mark per block, and several blocks for several views"),
    ("concat", "use several blocks with layout \"grid\""),
    ("hconcat", "use several blocks with layout \"grid\""),
    ("vconcat", "use several blocks with layout \"stack\""),
    ("facet", "use several blocks"),
    ("repeat", "use several blocks"),
    ("projection", "maps are not available yet"),
];
/// Document forms that state facts, so must cite where they came from.
const FACT_DOCUMENTS: &[&str] = &["metric", "table", "compare"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecError {
    pub path: String,
    pub message: String,
}

impl SpecError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// The marks a chart form may use.
pub fn marks_for(form: &str) -> &'static [&'static str] {
    match form {
        "bar" | "stacked-bar" | "grouped-bar" | "proportion" | "waterfall" | "funnel" => &["bar"],
        "line" => &["line"],
        "area" => &["area"],
        "scatter" => &["point", "circle"],
        "heatmap" => &["rect"],
        _ => &[],
    }
}

/// The encoding channels a chart form needs.
fn required_channels(form: &str) -> &'static [&'static str] {
    match form {
        "bar" | "line" | "area" | "scatter" | "waterfall" | "funnel" => &["x", "y"],
        "stacked-bar" | "grouped-bar" => &["x", "y", "color"],
        "heatmap" => &["x", "y", "color"],
        "proportion" => &["color"],
        _ => &[],
    }
}

struct Check {
    errors: Vec<SpecError>,
}

impl Check {
    fn push(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(SpecError::new(path, message));
    }

    /// A required string: present, non-empty after trimming, within `max`
    /// characters, and free of control characters.
    fn text(&mut self, value: Option<&Value>, path: &str, max: usize, required: bool) -> Option<String> {
        match value {
            None | Some(Value::Null) if !required => None,
            None | Some(Value::Null) => {
                self.push(path, "is required");
                None
            }
            Some(Value::String(text)) => {
                if required && text.trim().is_empty() {
                    self.push(path, "must not be empty");
                    return None;
                }
                if text.chars().count() > max {
                    self.push(path, format!("is longer than {max} characters"));
                }
                if has_control_characters(text) {
                    self.push(path, "contains control characters");
                }
                Some(text.clone())
            }
            Some(_) => {
                self.push(path, "must be a string");
                None
            }
        }
    }

    fn boolean(&mut self, value: Option<&Value>, path: &str) {
        if let Some(value) = value {
            if !value.is_boolean() {
                self.push(path, "must be true or false");
            }
        }
    }

    fn one_of(&mut self, value: Option<&Value>, path: &str, allowed: &[&str], required: bool) -> Option<String> {
        let text = self.text(value, path, 64, required)?;
        if !allowed.contains(&text.as_str()) {
            self.push(path, format!("must be one of: {}", allowed.join(", ")));
            return None;
        }
        Some(text)
    }

    fn array<'a>(&mut self, value: Option<&'a Value>, path: &str, min: usize, max: usize, required: bool) -> Option<&'a Vec<Value>> {
        match value {
            None | Some(Value::Null) if !required => None,
            None | Some(Value::Null) => {
                self.push(path, "is required");
                None
            }
            Some(Value::Array(items)) => {
                if items.len() < min {
                    self.push(path, format!("needs at least {min} item{}", if min == 1 { "" } else { "s" }));
                }
                if items.len() > max {
                    self.push(path, format!("has {} items; the limit is {max}", items.len()));
                }
                Some(items)
            }
            Some(_) => {
                self.push(path, "must be an array");
                None
            }
        }
    }

    fn object<'a>(&mut self, value: Option<&'a Value>, path: &str, required: bool) -> Option<&'a Map<String, Value>> {
        match value {
            None | Some(Value::Null) if !required => None,
            None | Some(Value::Null) => {
                self.push(path, "is required");
                None
            }
            Some(Value::Object(map)) => Some(map),
            Some(_) => {
                self.push(path, "must be an object");
                None
            }
        }
    }

    fn known_keys(&mut self, map: &Map<String, Value>, path: &str, allowed: &[&str]) {
        for key in map.keys() {
            if !allowed.contains(&key.as_str()) {
                let at = join(path, key);
                self.push(at, format!("is not a known field; use: {}", allowed.join(", ")));
            }
        }
    }
}

fn has_control_characters(text: &str) -> bool {
    text.chars()
        .any(|ch| (ch.is_control() && ch != '\n' && ch != '\t') || ch == '\u{7f}')
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn index(path: &str, at: usize) -> String {
    format!("{path}[{at}]")
}

/// Validate a spec. Empty means valid.
pub fn validate(spec: &Value) -> Vec<SpecError> {
    let mut check = Check { errors: Vec::new() };
    let size = serde_json::to_vec(spec).map(|bytes| bytes.len()).unwrap_or(usize::MAX);
    if size > MAX_SPEC_BYTES {
        check.push("", format!("is {size} bytes; the limit is {MAX_SPEC_BYTES}. Aggregate or sample the data"));
        return check.errors;
    }
    let Some(top) = spec.as_object() else {
        check.push("", "must be a JSON object");
        return check.errors;
    };
    check.known_keys(top, "", TOP_KEYS);
    match top.get("version") {
        Some(Value::Number(number)) if number.as_u64() == Some(1) => {}
        _ => check.push("version", "must be 1"),
    }
    check.text(top.get("title"), "title", 80, true);
    check.text(top.get("subtitle"), "subtitle", 160, false);
    check.one_of(top.get("layout"), "layout", &["stack", "grid"], false);
    check.boolean(top.get("redraw"), "redraw");
    if let Some(items) = check.array(top.get("followUps"), "followUps", 0, MAX_FOLLOW_UPS, false) {
        for (at, item) in items.iter().enumerate() {
            check.text(Some(item), &index("followUps", at), 80, true);
        }
    }
    if let Some(items) = check.array(top.get("notes"), "notes", 0, MAX_NOTES, false) {
        for (at, item) in items.iter().enumerate() {
            check.text(Some(item), &index("notes", at), 280, true);
        }
    }

    let sources = validate_sources(&mut check, top.get("sources"));
    if let Some(blocks) = check.array(top.get("blocks"), "blocks", 1, MAX_BLOCKS, true) {
        for (at, block) in blocks.iter().enumerate() {
            validate_block(&mut check, block, &index("blocks", at), &sources);
        }
    }
    check.errors
}

/// Declared sources, by id, with their kind.
struct Sources {
    kinds: std::collections::BTreeMap<String, String>,
}

impl Sources {
    fn kind(&self, id: &str) -> Option<&str> {
        self.kinds.get(id).map(String::as_str)
    }
}

fn validate_sources(check: &mut Check, value: Option<&Value>) -> Sources {
    let mut kinds = std::collections::BTreeMap::new();
    let Some(items) = check.array(value, "sources", 0, MAX_SOURCES, false) else {
        return Sources { kinds };
    };
    let mut pending_from = Vec::new();
    for (at, item) in items.iter().enumerate() {
        let path = index("sources", at);
        let Some(source) = check.object(Some(item), &path, true) else {
            continue;
        };
        check.known_keys(source, &path, &["id", "kind", "ref", "title", "from"]);
        let id = check.text(source.get("id"), &join(&path, "id"), 40, true);
        if let Some(id) = &id {
            if !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
                check.push(join(&path, "id"), "may contain only letters, digits, - and _");
            }
        }
        let kind = check.one_of(source.get("kind"), &join(&path, "kind"), SOURCE_KINDS, true);
        let reference = check.text(source.get("ref"), &join(&path, "ref"), 2000, true);
        check.text(source.get("title"), &join(&path, "title"), 120, false);
        match (kind.as_deref(), reference.as_deref()) {
            (Some("web"), Some(url)) if !(url.starts_with("https://") || url.starts_with("http://")) => {
                check.push(join(&path, "ref"), "must be the http(s) URL the value came from");
            }
            (Some("estimate"), Some(basis)) if basis.trim().chars().count() < 3 => {
                check.push(join(&path, "ref"), "must state the basis of the estimate");
            }
            _ => {}
        }
        if kind.as_deref() == Some("computed") {
            match check.array(source.get("from"), &join(&path, "from"), 1, MAX_SOURCES, true) {
                Some(from) => pending_from.push((path.clone(), id.clone(), from.clone())),
                None => {}
            }
        }
        if let (Some(id), Some(kind)) = (id, kind) {
            if kinds.insert(id.clone(), kind).is_some() {
                check.push(join(&path, "id"), format!("\"{id}\" is used by another source"));
            }
        }
    }
    for (path, own, from) in pending_from {
        for (at, reference) in from.iter().enumerate() {
            let at_path = index(&join(&path, "from"), at);
            match reference.as_str() {
                Some(reference) if Some(reference) == own.as_deref() => {
                    check.push(at_path, "a computed source cannot read itself");
                }
                Some(reference) if !kinds.contains_key(reference) => {
                    check.push(at_path, format!("names \"{reference}\", which is not a declared source id"));
                }
                Some(_) => {}
                None => check.push(at_path, "must be a source id"),
            }
        }
    }
    Sources { kinds }
}

fn validate_source_ids(check: &mut Check, value: Option<&Value>, path: &str, sources: &Sources, required: bool) -> Vec<String> {
    let min = usize::from(required);
    let Some(items) = check.array(value, path, min, MAX_SOURCES, required) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for (at, item) in items.iter().enumerate() {
        match item.as_str() {
            Some(id) if sources.kind(id).is_some() => ids.push(id.to_owned()),
            Some(id) => check.push(index(path, at), format!("names \"{id}\", which is not a declared source id")),
            None => check.push(index(path, at), "must be a source id"),
        }
    }
    ids
}

fn validate_block(check: &mut Check, value: &Value, path: &str, sources: &Sources) {
    let Some(block) = check.object(Some(value), path, true) else {
        return;
    };
    check.known_keys(block, path, BLOCK_KEYS);
    check.text(block.get("title"), &join(path, "title"), 80, false);
    let Some(family) = check.one_of(block.get("family"), &join(path, "family"), catalog::FAMILIES, true) else {
        return;
    };
    let Some(form) = check.text(block.get("form"), &join(path, "form"), 32, true) else {
        return;
    };
    let form_path = join(path, "form");
    match catalog::lookup(&family, &form) {
        None => {
            let known: Vec<_> = catalog::FORMS
                .iter()
                .filter(|info| info.family == family)
                .map(|info| info.form)
                .collect();
            check.push(form_path, format!("\"{form}\" is not a {family} form; {family} forms are: {}", known.join(", ")));
            return;
        }
        Some(info) if !info.available => {
            let available = catalog::available_in(&family);
            let hint = if available.is_empty() {
                format!("no {family} forms are available yet")
            } else {
                format!("available {family} forms: {}", available.join(", "))
            };
            check.push(form_path, format!("\"{form}\" is not available yet; {hint}"));
            return;
        }
        Some(_) => {}
    }
    match family.as_str() {
        "chart" => validate_chart(check, block, path, &form, sources),
        "document" => validate_document(check, block, path, &form, sources),
        "diagram" => validate_grid(check, block, path),
        _ => {}
    }
}

fn validate_chart(check: &mut Check, block: &Map<String, Value>, path: &str, form: &str, sources: &Sources) {
    let cited = validate_source_ids(check, block.get("sourceIds"), &join(path, "sourceIds"), sources, true);
    if let Some(colors) = check.object(block.get("colors"), &join(path, "colors"), false) {
        for (series, color) in colors {
            check.one_of(Some(color), &join(&join(path, "colors"), series), COLORS, true);
        }
    }
    let vega_path = join(path, "vegaLite");
    let Some(vega) = check.object(block.get("vegaLite"), &vega_path, true) else {
        return;
    };
    forbid_vega_features(check, &Value::Object(vega.clone()), &vega_path);

    let mark_path = join(&vega_path, "mark");
    let mark = match vega.get("mark") {
        Some(Value::String(mark)) => Some(mark.clone()),
        Some(Value::Object(mark)) => mark.get("type").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    };
    let allowed = marks_for(form);
    match mark {
        None => check.push(mark_path, format!("is required; a {form} uses mark \"{}\"", allowed.join("\" or \""))),
        Some(mark) if !allowed.contains(&mark.as_str()) => {
            check.push(mark_path, format!("\"{mark}\" does not draw a {form}; use \"{}\"", allowed.join("\" or \"")));
        }
        Some(_) => {}
    }

    let data_path = join(&vega_path, "data");
    let rows = match check.object(vega.get("data"), &data_path, true) {
        Some(data) => {
            if data.contains_key("url") || data.contains_key("name") {
                // Already reported by the forbidden-feature scan for url; name
                // is a dataset reference.
                if data.contains_key("name") {
                    check.push(join(&data_path, "name"), "pass the rows inline in data.values");
                }
            }
            check.array(data.get("values"), &join(&data_path, "values"), 1, MAX_ROWS, true)
        }
        None => None,
    };
    let mut fields = std::collections::BTreeSet::new();
    let mut estimates = false;
    if let Some(rows) = rows {
        let values_path = join(&data_path, "values");
        for (at, row) in rows.iter().enumerate() {
            let row_path = index(&values_path, at);
            let Some(row) = row.as_object() else {
                check.push(row_path, "must be an object of field: value");
                continue;
            };
            for (key, value) in row {
                fields.insert(key.clone());
                match value {
                    Value::String(text) => {
                        if text.chars().count() > 200 {
                            check.push(join(&row_path, key), "is longer than 200 characters");
                        } else if has_control_characters(text) {
                            check.push(join(&row_path, key), "contains control characters");
                        }
                    }
                    Value::Number(number) if number.as_f64().is_some_and(f64::is_finite) => {}
                    Value::Bool(flag) => {
                        if key == "estimate" && *flag {
                            estimates = true;
                        }
                    }
                    Value::Null => {}
                    _ => check.push(join(&row_path, key), "must be a string, number, true/false or null"),
                }
            }
        }
    }
    if estimates && !cited.iter().any(|id| sources.kind(id) == Some("estimate")) {
        check.push(join(path, "sourceIds"), "rows marked estimate: true need an estimate source stating the basis");
    }

    let encoding_path = join(&vega_path, "encoding");
    let Some(encoding) = check.object(vega.get("encoding"), &encoding_path, true) else {
        return;
    };
    for channel in required_channels(form) {
        let mut present = encoding.contains_key(*channel);
        // A proportion reads its size from x or theta, whichever is given.
        if form == "proportion" && *channel == "color" {
            present = present && (encoding.contains_key("x") || encoding.contains_key("theta"));
            if !present {
                check.push(encoding_path.clone(), "a proportion needs color (the parts) and x or theta (their size)");
                continue;
            }
        }
        if !present {
            check.push(join(&encoding_path, channel), format!("is required for a {form}"));
        }
    }
    for (channel, definition) in encoding {
        let channel_path = join(&encoding_path, channel);
        let Some(definition) = definition.as_object() else {
            if channel == "tooltip" && definition.is_array() {
                continue;
            }
            check.push(channel_path, "must be an object like {\"field\": \"…\", \"type\": \"…\"}");
            continue;
        };
        if let Some(value) = definition.get("value") {
            if channel == "color" {
                check.one_of(Some(value), &join(&channel_path, "value"), COLORS, true);
            }
        }
        if let Some(kind) = definition.get("type") {
            check.one_of(Some(kind), &join(&channel_path, "type"), ENCODING_TYPES, true);
        }
        let aggregate = definition.get("aggregate").and_then(Value::as_str);
        if let Some(value) = definition.get("aggregate") {
            check.one_of(Some(value), &join(&channel_path, "aggregate"), AGGREGATES, true);
        }
        if let Some(scale) = definition.get("scale").and_then(Value::as_object) {
            for key in ["range", "scheme"] {
                if scale.contains_key(key) {
                    check.push(join(&join(&channel_path, "scale"), key), "colours come from the block's colors map, not the scale");
                }
            }
        }
        match definition.get("field") {
            Some(Value::String(field)) => {
                if !fields.is_empty() && !fields.contains(field) {
                    check.push(join(&channel_path, "field"), format!("\"{field}\" is not a field of any row in data.values"));
                }
            }
            Some(_) => check.push(join(&channel_path, "field"), "must be a field name"),
            None if aggregate == Some("count") || definition.contains_key("value") || channel == "tooltip" => {}
            None => check.push(join(&channel_path, "field"), "is required"),
        }
    }
}

/// Reject Vega-Lite features this build does not draw, wherever they appear.
fn forbid_vega_features(check: &mut Check, value: &Value, path: &str) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let at = join(path, key);
                if let Some((_, fix)) = FORBIDDEN_VEGA.iter().find(|(name, _)| name == key) {
                    check.push(at, format!("is not supported: {fix}"));
                    continue;
                }
                // Row values are data, not configuration; the row checks
                // cover them.
                if key == "values" {
                    continue;
                }
                forbid_vega_features(check, inner, &at);
            }
        }
        Value::Array(items) => {
            for (at, item) in items.iter().enumerate() {
                forbid_vega_features(check, item, &index(path, at));
            }
        }
        _ => {}
    }
}

fn scalar_cell(check: &mut Check, value: &Value, path: &str, max: usize) {
    match value {
        Value::String(_) => {
            check.text(Some(value), path, max, false);
        }
        Value::Number(_) | Value::Bool(_) | Value::Null => {}
        _ => check.push(path, "must be a string, number, true/false or null"),
    }
}

fn validate_document(check: &mut Check, block: &Map<String, Value>, path: &str, form: &str, sources: &Sources) {
    let fact = FACT_DOCUMENTS.contains(&form);
    let cited = validate_source_ids(check, block.get("sourceIds"), &join(path, "sourceIds"), sources, fact);
    let content_path = join(path, "content");
    let Some(content) = check.object(block.get("content"), &content_path, true) else {
        return;
    };
    let mut estimates = false;
    match form {
        "metric" => {
            check.known_keys(content, &content_path, &["items"]);
            let items_path = join(&content_path, "items");
            for (at, item) in check.array(content.get("items"), &items_path, 1, 6, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&items_path, at);
                let Some(item) = check.object(Some(item), &at_path, true) else { continue };
                check.known_keys(item, &at_path, &["label", "value", "delta", "tone", "note", "estimate"]);
                check.text(item.get("label"), &join(&at_path, "label"), 60, true);
                match item.get("value") {
                    Some(Value::Number(_)) => {}
                    other => {
                        check.text(other, &join(&at_path, "value"), 24, true);
                    }
                }
                check.text(item.get("delta"), &join(&at_path, "delta"), 24, false);
                check.one_of(item.get("tone"), &join(&at_path, "tone"), &["positive", "negative", "neutral"], false);
                check.text(item.get("note"), &join(&at_path, "note"), 120, false);
                check.boolean(item.get("estimate"), &join(&at_path, "estimate"));
                estimates |= item.get("estimate") == Some(&Value::Bool(true));
            }
        }
        "cards" => {
            check.known_keys(content, &content_path, &["items"]);
            let items_path = join(&content_path, "items");
            for (at, item) in check.array(content.get("items"), &items_path, 1, 12, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&items_path, at);
                let Some(item) = check.object(Some(item), &at_path, true) else { continue };
                check.known_keys(item, &at_path, &["title", "body", "meta", "tags"]);
                check.text(item.get("title"), &join(&at_path, "title"), 80, true);
                check.text(item.get("body"), &join(&at_path, "body"), 400, false);
                check.text(item.get("meta"), &join(&at_path, "meta"), 80, false);
                if let Some(tags) = check.array(item.get("tags"), &join(&at_path, "tags"), 0, 6, false) {
                    for (tag_at, tag) in tags.iter().enumerate() {
                        check.text(Some(tag), &index(&join(&at_path, "tags"), tag_at), 24, true);
                    }
                }
            }
        }
        "compare" => {
            check.known_keys(content, &content_path, &["criteria", "options"]);
            let criteria_path = join(&content_path, "criteria");
            let criteria: Vec<String> = check
                .array(content.get("criteria"), &criteria_path, 1, 12, true)
                .cloned()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .filter_map(|(at, item)| check.text(Some(item), &index(&criteria_path, at), 60, true))
                .collect();
            let options_path = join(&content_path, "options");
            for (at, option) in check.array(content.get("options"), &options_path, 2, 5, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&options_path, at);
                let Some(option) = check.object(Some(option), &at_path, true) else { continue };
                check.known_keys(option, &at_path, &["name", "highlight", "summary", "values"]);
                check.text(option.get("name"), &join(&at_path, "name"), 40, true);
                check.boolean(option.get("highlight"), &join(&at_path, "highlight"));
                check.text(option.get("summary"), &join(&at_path, "summary"), 160, false);
                if let Some(values) = check.object(option.get("values"), &join(&at_path, "values"), true) {
                    for (criterion, value) in values {
                        let value_path = join(&join(&at_path, "values"), criterion);
                        if !criteria.is_empty() && !criteria.contains(criterion) {
                            check.push(value_path.clone(), "is not one of content.criteria");
                        }
                        scalar_cell(check, value, &value_path, 80);
                    }
                }
            }
        }
        "table" => {
            check.known_keys(content, &content_path, &["columns", "rows"]);
            let columns_path = join(&content_path, "columns");
            let mut keys = Vec::new();
            for (at, column) in check.array(content.get("columns"), &columns_path, 1, 8, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&columns_path, at);
                let Some(column) = check.object(Some(column), &at_path, true) else { continue };
                check.known_keys(column, &at_path, &["key", "label", "align", "format"]);
                if let Some(key) = check.text(column.get("key"), &join(&at_path, "key"), 40, true) {
                    keys.push(key);
                }
                check.text(column.get("label"), &join(&at_path, "label"), 40, true);
                check.one_of(column.get("align"), &join(&at_path, "align"), &["left", "right", "center"], false);
                check.one_of(column.get("format"), &join(&at_path, "format"), &["text", "number", "percent", "currency", "compact"], false);
            }
            let rows_path = join(&content_path, "rows");
            for (at, row) in check.array(content.get("rows"), &rows_path, 1, 200, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&rows_path, at);
                let Some(row) = check.object(Some(row), &at_path, true) else { continue };
                for (key, value) in row {
                    if key == "estimate" {
                        check.boolean(Some(value), &join(&at_path, key));
                        estimates |= value == &Value::Bool(true);
                        continue;
                    }
                    if !keys.is_empty() && !keys.contains(key) {
                        check.push(join(&at_path, key), "is not a column key");
                    }
                    scalar_cell(check, value, &join(&at_path, key), 200);
                }
            }
        }
        "callout" => {
            check.known_keys(content, &content_path, &["tone", "title", "body"]);
            check.one_of(content.get("tone"), &join(&content_path, "tone"), &["info", "warning", "success", "danger", "neutral"], true);
            check.text(content.get("title"), &join(&content_path, "title"), 80, false);
            check.text(content.get("body"), &join(&content_path, "body"), 600, true);
        }
        "steps" => {
            check.known_keys(content, &content_path, &["items"]);
            let items_path = join(&content_path, "items");
            for (at, item) in check.array(content.get("items"), &items_path, 2, 12, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&items_path, at);
                let Some(item) = check.object(Some(item), &at_path, true) else { continue };
                check.known_keys(item, &at_path, &["title", "body"]);
                check.text(item.get("title"), &join(&at_path, "title"), 80, true);
                check.text(item.get("body"), &join(&at_path, "body"), 400, false);
            }
        }
        "checklist" => {
            check.known_keys(content, &content_path, &["items"]);
            let items_path = join(&content_path, "items");
            for (at, item) in check.array(content.get("items"), &items_path, 1, 20, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&items_path, at);
                let Some(item) = check.object(Some(item), &at_path, true) else { continue };
                check.known_keys(item, &at_path, &["text", "done"]);
                check.text(item.get("text"), &join(&at_path, "text"), 160, true);
                match item.get("done") {
                    Some(Value::Bool(_)) => {}
                    _ => check.push(join(&at_path, "done"), "must be true or false"),
                }
            }
        }
        "findings" => {
            check.known_keys(content, &content_path, &["items"]);
            let items_path = join(&content_path, "items");
            for (at, item) in check.array(content.get("items"), &items_path, 1, 8, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&items_path, at);
                let Some(item) = check.object(Some(item), &at_path, true) else { continue };
                check.known_keys(item, &at_path, &["text", "sourceIds", "estimate"]);
                check.text(item.get("text"), &join(&at_path, "text"), 280, true);
                let ids = validate_source_ids(check, item.get("sourceIds"), &join(&at_path, "sourceIds"), sources, true);
                check.boolean(item.get("estimate"), &join(&at_path, "estimate"));
                if item.get("estimate") == Some(&Value::Bool(true)) && !ids.iter().any(|id| sources.kind(id) == Some("estimate")) {
                    check.push(join(&at_path, "sourceIds"), "an estimate finding needs an estimate source stating the basis");
                }
            }
        }
        "pros-cons" => {
            check.known_keys(content, &content_path, &["pros", "cons"]);
            for side in ["pros", "cons"] {
                let side_path = join(&content_path, side);
                for (at, item) in check.array(content.get(side), &side_path, 1, 8, true).cloned().unwrap_or_default().iter().enumerate() {
                    check.text(Some(item), &index(&side_path, at), 160, true);
                }
            }
        }
        "glossary" => {
            check.known_keys(content, &content_path, &["items"]);
            let items_path = join(&content_path, "items");
            for (at, item) in check.array(content.get("items"), &items_path, 1, 20, true).cloned().unwrap_or_default().iter().enumerate() {
                let at_path = index(&items_path, at);
                let Some(item) = check.object(Some(item), &at_path, true) else { continue };
                check.known_keys(item, &at_path, &["term", "definition"]);
                check.text(item.get("term"), &join(&at_path, "term"), 40, true);
                check.text(item.get("definition"), &join(&at_path, "definition"), 280, true);
            }
        }
        _ => {}
    }
    if estimates && !cited.iter().any(|id| sources.kind(id) == Some("estimate")) {
        check.push(join(path, "sourceIds"), "values marked estimate need an estimate source stating the basis");
    }
}

/// The `grid` diagram is the existing `DiagramSpec`, checked by the same rules
/// as `isValidDiagramSpec` in `src/components/DiagramFigure.tsx`.
fn validate_grid(check: &mut Check, block: &Map<String, Value>, path: &str) {
    let graph_path = join(path, "graph");
    let Some(graph) = check.object(block.get("graph"), &graph_path, true) else {
        return;
    };
    check.known_keys(graph, &graph_path, &["nodes", "edges", "caption", "ariaLabel"]);
    check.text(graph.get("caption"), &join(&graph_path, "caption"), 200, true);
    check.text(graph.get("ariaLabel"), &join(&graph_path, "ariaLabel"), 200, true);
    let emphasis = ["default", "muted", "active"];
    let mut ids = std::collections::BTreeSet::new();
    let nodes_path = join(&graph_path, "nodes");
    for (at, node) in check.array(graph.get("nodes"), &nodes_path, 1, MAX_DIAGRAM_NODES, true).cloned().unwrap_or_default().iter().enumerate() {
        let at_path = index(&nodes_path, at);
        let Some(node) = check.object(Some(node), &at_path, true) else { continue };
        check.known_keys(node, &at_path, &["id", "label", "row", "col", "emphasis", "marker", "labelSide"]);
        if let Some(id) = check.text(node.get("id"), &join(&at_path, "id"), 40, true) {
            if !ids.insert(id.clone()) {
                check.push(join(&at_path, "id"), format!("\"{id}\" is used by another node"));
            }
        }
        check.text(node.get("label"), &join(&at_path, "label"), 60, false);
        for axis in ["row", "col"] {
            match node.get(axis).and_then(Value::as_u64) {
                Some(cell) if cell <= 60 => {}
                _ => check.push(join(&at_path, axis), "must be a whole number from 0 to 60"),
            }
        }
        check.one_of(node.get("emphasis"), &join(&at_path, "emphasis"), &emphasis, false);
        check.one_of(node.get("marker"), &join(&at_path, "marker"), &["none", "checkpoint", "tip", "continues"], false);
        check.one_of(node.get("labelSide"), &join(&at_path, "labelSide"), &["right", "below"], false);
    }
    let edges_path = join(&graph_path, "edges");
    for (at, edge) in check.array(graph.get("edges"), &edges_path, 0, MAX_DIAGRAM_NODES * 3, true).cloned().unwrap_or_default().iter().enumerate() {
        let at_path = index(&edges_path, at);
        let Some(edge) = check.object(Some(edge), &at_path, true) else { continue };
        check.known_keys(edge, &at_path, &["from", "to", "curve", "emphasis"]);
        for end in ["from", "to"] {
            if let Some(id) = check.text(edge.get(end), &join(&at_path, end), 40, true) {
                if !ids.contains(&id) {
                    check.push(join(&at_path, end), format!("names \"{id}\", which is not a node id"));
                }
            }
        }
        check.boolean(edge.get("curve"), &join(&at_path, "curve"));
        check.one_of(edge.get("emphasis"), &join(&at_path, "emphasis"), &emphasis, false);
    }
}

/// Every chart block's rows, in order. Used by lint and the repeat guard.
pub fn chart_rows(block: &Value) -> &[Value] {
    block
        .pointer("/vegaLite/data/values")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn bar() -> Value {
        json!({
            "version": 1,
            "title": "Spend by harness",
            "sources": [{"id": "ledger", "kind": "tool", "ref": "toolu_01"}],
            "blocks": [{
                "family": "chart",
                "form": "bar",
                "sourceIds": ["ledger"],
                "vegaLite": {
                    "mark": "bar",
                    "data": {"values": [
                        {"harness": "Claude", "usd": 1.96},
                        {"harness": "Codex", "usd": 1.18},
                        {"harness": "OpenCode", "usd": 0.27}
                    ]},
                    "encoding": {
                        "x": {"field": "harness", "type": "nominal"},
                        "y": {"field": "usd", "type": "quantitative"}
                    }
                }
            }]
        })
    }

    fn paths(spec: &Value) -> Vec<String> {
        validate(spec).into_iter().map(|error| error.path).collect()
    }

    #[test]
    fn a_minimal_bar_chart_with_a_source_is_valid() {
        assert_eq!(validate(&bar()), vec![]);
    }

    #[test]
    fn version_must_be_1() {
        let mut spec = bar();
        spec["version"] = json!(2);
        assert_eq!(paths(&spec), vec!["version"]);
    }

    #[test]
    fn title_is_required_and_capped_at_80_chars() {
        let mut spec = bar();
        spec["title"] = json!("x".repeat(81));
        assert_eq!(paths(&spec), vec!["title"]);
        spec.as_object_mut().unwrap().remove("title");
        assert_eq!(paths(&spec), vec!["title"]);
    }

    #[test]
    fn blocks_must_number_between_1_and_6() {
        let mut spec = bar();
        spec["blocks"] = json!([]);
        assert_eq!(paths(&spec), vec!["blocks"]);
        let block = bar()["blocks"][0].clone();
        spec["blocks"] = json!(vec![block; 7]);
        assert_eq!(paths(&spec), vec!["blocks"]);
    }

    #[test]
    fn an_unknown_family_is_rejected_with_its_path() {
        let mut spec = bar();
        spec["blocks"][0]["family"] = json!("poster");
        assert_eq!(paths(&spec), vec!["blocks[0].family"]);
    }

    #[test]
    fn a_form_outside_its_family_is_rejected() {
        let mut spec = bar();
        spec["blocks"][0]["form"] = json!("steps");
        let errors = validate(&spec);
        assert_eq!(errors[0].path, "blocks[0].form");
        assert!(errors[0].message.contains("chart forms are"), "{errors:?}");
    }

    #[test]
    fn a_form_not_available_yet_names_the_available_ones() {
        let mut spec = bar();
        spec["blocks"][0]["family"] = json!("diagram");
        spec["blocks"][0]["form"] = json!("sequence");
        let errors = validate(&spec);
        assert_eq!(errors[0].path, "blocks[0].form");
        assert!(errors[0].message.contains("not available yet"), "{errors:?}");
        assert!(errors[0].message.contains("grid"), "{errors:?}");
    }

    #[test]
    fn chart_data_must_be_inline_values() {
        let mut spec = bar();
        spec["blocks"][0]["vegaLite"]["data"] = json!({"url": "https://example.com/data.csv"});
        let found = paths(&spec);
        assert!(found.contains(&"blocks[0].vegaLite.data.url".to_owned()), "{found:?}");
        assert!(found.contains(&"blocks[0].vegaLite.data.values".to_owned()), "{found:?}");
    }

    #[test]
    fn vega_lite_config_usermeta_params_transform_and_href_are_rejected() {
        for key in ["config", "usermeta", "params", "transform"] {
            let mut spec = bar();
            spec["blocks"][0]["vegaLite"][key] = json!({});
            assert_eq!(paths(&spec), vec![format!("blocks[0].vegaLite.{key}")], "{key}");
        }
        let mut spec = bar();
        spec["blocks"][0]["vegaLite"]["encoding"]["href"] = json!({"field": "harness"});
        assert_eq!(paths(&spec), vec!["blocks[0].vegaLite.encoding.href"]);
    }

    #[test]
    fn chart_rows_must_be_objects_of_scalars() {
        let mut spec = bar();
        spec["blocks"][0]["vegaLite"]["data"]["values"][1] = json!(["Codex", 1.18]);
        spec["blocks"][0]["vegaLite"]["data"]["values"][2]["usd"] = json!({"nested": 1});
        assert_eq!(
            paths(&spec),
            vec![
                "blocks[0].vegaLite.data.values[1]",
                "blocks[0].vegaLite.data.values[2].usd"
            ]
        );
    }

    #[test]
    fn chart_rows_are_capped_at_5000() {
        let mut spec = bar();
        let rows: Vec<Value> = (0..5001).map(|at| json!({"harness": at.to_string(), "usd": at})).collect();
        spec["blocks"][0]["vegaLite"]["data"]["values"] = json!(rows);
        assert_eq!(paths(&spec), vec!["blocks[0].vegaLite.data.values"]);
    }

    #[test]
    fn an_encoding_field_missing_from_every_row_is_rejected() {
        let mut spec = bar();
        spec["blocks"][0]["vegaLite"]["encoding"]["y"]["field"] = json!("cost");
        assert_eq!(paths(&spec), vec!["blocks[0].vegaLite.encoding.y.field"]);
    }

    #[test]
    fn the_mark_must_agree_with_the_form() {
        let mut spec = bar();
        spec["blocks"][0]["form"] = json!("line");
        let errors = validate(&spec);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].path, "blocks[0].vegaLite.mark");
        assert!(errors[0].message.contains("\"line\""));
    }

    #[test]
    fn colors_must_come_from_the_enum() {
        let mut spec = bar();
        spec["blocks"][0]["colors"] = json!({"Claude": "#ff0000"});
        assert_eq!(paths(&spec), vec!["blocks[0].colors.Claude"]);
        spec["blocks"][0]["colors"] = json!({"Claude": "claude"});
        assert_eq!(paths(&spec), Vec::<String>::new());
        spec["blocks"][0]["vegaLite"]["encoding"]["color"] = json!({"value": "red"});
        assert_eq!(paths(&spec), vec!["blocks[0].vegaLite.encoding.color.value"]);
    }

    #[test]
    fn fact_blocks_must_cite_declared_sources() {
        let mut spec = bar();
        spec["blocks"][0].as_object_mut().unwrap().remove("sourceIds");
        assert_eq!(paths(&spec), vec!["blocks[0].sourceIds"]);
        let mut spec = bar();
        spec["blocks"][0]["sourceIds"] = json!(["nowhere"]);
        assert_eq!(paths(&spec), vec!["blocks[0].sourceIds[0]"]);
        for form in ["metric", "table", "compare"] {
            let spec = json!({"version": 1, "title": "t", "blocks": [{"family": "document", "form": form, "content": {}}]});
            assert!(paths(&spec).contains(&"blocks[0].sourceIds".to_owned()), "{form}");
        }
    }

    #[test]
    fn findings_items_cite_per_item() {
        let spec = json!({
            "version": 1, "title": "t",
            "sources": [{"id": "a", "kind": "web", "ref": "https://example.com"}],
            "blocks": [{"family": "document", "form": "findings", "content": {"items": [
                {"text": "cited", "sourceIds": ["a"]},
                {"text": "uncited"}
            ]}}]
        });
        assert_eq!(paths(&spec), vec!["blocks[0].content.items[1].sourceIds"]);
    }

    #[test]
    fn web_sources_must_be_http_urls() {
        let mut spec = bar();
        spec["sources"][0] = json!({"id": "ledger", "kind": "web", "ref": "iea.org"});
        assert_eq!(paths(&spec), vec!["sources[0].ref"]);
    }

    #[test]
    fn computed_sources_must_name_existing_inputs() {
        let mut spec = bar();
        spec["sources"] = json!([
            {"id": "ledger", "kind": "computed", "ref": "a / b", "from": ["a", "ledger"]}
        ]);
        assert_eq!(paths(&spec), vec!["sources[0].from[0]", "sources[0].from[1]"]);
    }

    #[test]
    fn estimate_sources_must_state_a_basis() {
        let mut spec = bar();
        spec["sources"][0] = json!({"id": "ledger", "kind": "estimate", "ref": "?"});
        assert_eq!(paths(&spec), vec!["sources[0].ref"]);
    }

    #[test]
    fn rows_marked_estimate_need_an_estimate_source() {
        let mut spec = bar();
        spec["blocks"][0]["vegaLite"]["data"]["values"][2]["estimate"] = json!(true);
        assert_eq!(paths(&spec), vec!["blocks[0].sourceIds"]);
        spec["sources"].as_array_mut().unwrap().push(json!({"id": "guess", "kind": "estimate", "ref": "one source, mid-year"}));
        spec["blocks"][0]["sourceIds"] = json!(["ledger", "guess"]);
        assert_eq!(paths(&spec), Vec::<String>::new());
    }

    #[test]
    fn source_ids_must_be_unique() {
        let mut spec = bar();
        spec["sources"].as_array_mut().unwrap().push(json!({"id": "ledger", "kind": "user", "ref": "entry-1"}));
        assert_eq!(paths(&spec), vec!["sources[1].id"]);
    }

    #[test]
    fn follow_ups_are_capped_at_4_of_80_chars() {
        let mut spec = bar();
        spec["followUps"] = json!(["a", "b", "c", "d", "e"]);
        assert_eq!(paths(&spec), vec!["followUps"]);
        spec["followUps"] = json!(["x".repeat(81)]);
        assert_eq!(paths(&spec), vec!["followUps[0]"]);
    }

    #[test]
    fn grid_diagrams_reuse_diagram_spec() {
        let spec = json!({
            "version": 1, "title": "t",
            "blocks": [{"family": "diagram", "form": "grid", "graph": {
                "caption": "c", "ariaLabel": "a",
                "nodes": [
                    {"id": "a", "row": 0, "col": 0},
                    {"id": "a", "row": -1, "col": 0, "emphasis": "loud", "marker": "star"}
                ],
                "edges": [{"from": "a", "to": "b"}]
            }}]
        });
        assert_eq!(
            paths(&spec),
            vec![
                "blocks[0].graph.nodes[1].id",
                "blocks[0].graph.nodes[1].row",
                "blocks[0].graph.nodes[1].emphasis",
                "blocks[0].graph.nodes[1].marker",
                "blocks[0].graph.edges[0].to"
            ]
        );
    }

    #[test]
    fn strings_with_control_characters_are_rejected() {
        let mut spec = bar();
        spec["title"] = json!("Spend\u{0007}");
        assert_eq!(paths(&spec), vec!["title"]);
    }

    #[test]
    fn a_spec_over_256_kib_is_rejected() {
        let mut spec = bar();
        spec["notes"] = json!(["x".repeat(MAX_SPEC_BYTES)]);
        assert_eq!(paths(&spec), vec![""]);
    }

    #[test]
    fn unknown_top_level_and_block_fields_are_named() {
        let mut spec = bar();
        spec["type"] = json!("chart");
        spec["blocks"][0]["data"] = json!([]);
        assert_eq!(paths(&spec), vec!["type", "blocks[0].data"]);
    }

    #[test]
    fn the_shared_fixtures_agree_with_this_validator() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/visualize/specs.json");
        let fixtures: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("specs.json exists"))
                .expect("specs.json parses");
        assert!(fixtures.len() >= 20, "keep the shared fixtures substantial");
        for fixture in fixtures {
            let name = fixture["name"].as_str().unwrap_or("?");
            let errors = validate(&fixture["spec"]);
            let valid = fixture["valid"].as_bool().expect("valid flag");
            assert_eq!(errors.is_empty(), valid, "{name}: {errors:?}");
            if let Some(expected) = fixture.get("errorPaths").and_then(Value::as_array) {
                let mut got: Vec<_> = errors.iter().map(|error| error.path.clone()).collect();
                let mut want: Vec<_> = expected.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                got.sort();
                want.sort();
                assert_eq!(got, want, "{name}");
            }
        }
    }

    #[test]
    fn every_available_form_has_a_valid_golden() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/visualize/specs.json");
        let fixtures: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("specs.json exists")).unwrap();
        for info in catalog::available() {
            let covered = fixtures.iter().any(|fixture| {
                fixture["valid"] == json!(true)
                    && fixture["spec"]["blocks"].as_array().is_some_and(|blocks| {
                        blocks.iter().any(|block| block["family"] == json!(info.family) && block["form"] == json!(info.form))
                    })
            });
            assert!(covered, "no valid golden for {}/{}", info.family, info.form);
        }
    }
}
