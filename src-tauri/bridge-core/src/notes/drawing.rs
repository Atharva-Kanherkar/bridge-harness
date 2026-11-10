//! Excalidraw drawings inside a page, as agents see and change them.
//!
//! A drawing is an ordinary fenced block, ```` ```excalidraw ````, whose body
//! is an Excalidraw scene (`{"type":"excalidraw","elements":[...]}`), the same
//! JSON an `.excalidraw` file holds. The editor draws it; the `.md` file keeps
//! it as text.
//!
//! Elements come in two shapes. One a person drew carries the whole of
//! Excalidraw's element (it has a `version`). One an agent added through
//! `notes_draw` is a skeleton in Excalidraw's own `ExcalidrawElementSkeleton`
//! form: a container with `label: {text}`, an arrow with `start: {id}` and
//! `end: {id}`, and no `version`. The editor completes skeletons when it loads
//! the scene (`convertToExcalidrawElements`) and saves them whole once the
//! person edits the drawing. Everything here reads both.
//!
//! Agents never see the raw JSON: [`for_agent`] swaps each body for a marker,
//! and [`describe`] says what the drawing holds in a few plain lines, by
//! element id, so an agent can answer about it and name what to change.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Map, Value};

pub const FENCE: &str = "excalidraw";
/// Lines one drawing's description may take before it is clipped.
const MAX_LINES: usize = 160;
const LABEL_CHARS: usize = 160;
const DEFAULT_WIDTH: f64 = 180.0;
const DEFAULT_HEIGHT: f64 = 80.0;
/// Room between laid-out shapes, and between a shape and an arrow's end.
const COLUMN_GAP: f64 = 90.0;
const ROW_GAP: f64 = 60.0;
const ARROW_GAP: f64 = 6.0;
/// The most elements one `notes_draw` call may add, change or remove.
pub const MAX_OPS: usize = 80;

/// One ```` ```excalidraw ```` block in a page's Markdown, by byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    /// The whole block, opening and closing fence lines included.
    pub start: usize,
    pub end: usize,
    /// The body between the fence lines.
    pub body_start: usize,
    pub body_end: usize,
}

/// Every drawing in `markdown`, in page order. Follows CommonMark's fence
/// rules: up to three spaces of indent, backticks or tildes, and a closing run
/// of the same character at least as long. An unclosed fence runs to the end.
pub fn find(markdown: &str) -> Vec<Fence> {
    let mut fences = Vec::new();
    let mut offset = 0;
    let mut open: Option<(char, usize, usize, bool)> = None; // marker, length, start, is drawing
    let mut body_start = 0;
    for line in markdown.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let indent = text.len() - text.trim_start_matches(' ').len();
        let trimmed = &text[indent.min(text.len())..];
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = marker.map(|m| trimmed.chars().take_while(|c| *c == m).count()).unwrap_or(0);
        match open {
            None if indent <= 3 && run >= 3 => {
                let marker = marker.unwrap();
                let info = trimmed[run..].trim();
                // A backtick fence's info string may not hold a backtick.
                if marker == '`' && info.contains('`') {
                    offset += line.len();
                    continue;
                }
                let lang = info.split_whitespace().next().unwrap_or("");
                open = Some((marker, run, offset, lang.eq_ignore_ascii_case(FENCE)));
                body_start = offset + line.len();
            }
            Some((m, length, start, drawing)) if indent <= 3 && marker == Some(m) && run >= length && trimmed[run..].trim().is_empty() => {
                if drawing {
                    fences.push(Fence { start, end: offset + line.len(), body_start, body_end: offset });
                }
                open = None;
            }
            _ => {}
        }
        offset += line.len();
    }
    if let Some((_, _, start, true)) = open {
        fences.push(Fence { start, end: markdown.len(), body_start: body_start.min(markdown.len()), body_end: markdown.len() });
    }
    fences
}

/// A scene from a fence body. An empty body is an empty scene.
pub fn parse(body: &str) -> Result<Value, String> {
    if body.trim().is_empty() {
        return Ok(empty_scene());
    }
    let scene: Value = serde_json::from_str(body).map_err(|error| format!("it is not valid Excalidraw JSON ({error})"))?;
    match scene.get("elements") {
        Some(Value::Array(_)) => Ok(scene),
        _ if scene.is_array() => Ok(json!({"type": "excalidraw", "version": 2, "source": "bridge", "elements": scene})),
        _ => Err("it has no elements list".into()),
    }
}

pub fn empty_scene() -> Value {
    json!({"type": "excalidraw", "version": 2, "source": "bridge", "elements": [], "appState": {"viewBackgroundColor": "#ffffff"}, "files": {}})
}

/// A scene as a fence body: one element per line, so a change to one element
/// is a one-line change in the page's history and in git. The editor writes
/// the same layout (`serializeScene`).
pub fn serialize(scene: &Value) -> String {
    let scene = &whole_numbers(scene.clone());
    let source = scene.get("source").cloned().unwrap_or_else(|| json!("bridge"));
    let mut out = format!("{{\"type\":\"excalidraw\",\"version\":2,\"source\":{source},\"elements\":[");
    let elements = scene["elements"].as_array().cloned().unwrap_or_default();
    for (index, element) in elements.iter().enumerate() {
        out.push_str(if index == 0 { "\n" } else { ",\n" });
        out.push_str(&element.to_string());
    }
    out.push_str(if elements.is_empty() { "]" } else { "\n]" });
    if let Value::Object(object) = scene {
        for (key, value) in object.iter().filter(|(key, _)| !matches!(key.as_str(), "type" | "version" | "source" | "elements")) {
            out.push_str(&format!(",{}:{value}", json!(key)));
        }
    }
    out.push('}');
    out
}

/// Whole numbers written as integers (`40`, not `40.0`), as JavaScript writes
/// them, so a scene reads the same whichever side wrote it last.
fn whole_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() && float.fract() == 0.0 && float.abs() < 9.0e15 => json!(float as i64),
            _ => Value::Number(number),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(whole_numbers).collect()),
        Value::Object(object) => Value::Object(object.into_iter().map(|(key, value)| (key, whole_numbers(value))).collect()),
        other => other,
    }
}

/// The page as an agent reads it: each drawing's body swapped for a marker,
/// and each drawing described, in page order.
pub fn for_agent(markdown: &str) -> (String, Vec<Value>) {
    let fences = find(markdown);
    if fences.is_empty() {
        return (markdown.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(markdown.len());
    let mut drawings = Vec::new();
    let mut cursor = 0;
    for (index, fence) in fences.iter().enumerate() {
        let number = index + 1;
        out.push_str(&markdown[cursor..fence.body_start]);
        let body = &markdown[fence.body_start..fence.body_end];
        let description = match parse(body) {
            Ok(scene) => describe(&scene),
            Err(error) => format!("Drawing {number} could not be read: {error}."),
        };
        out.push_str(&format!("[drawing {number}: see drawings[{index}]; change it with notes_draw, never notes_edit]\n"));
        drawings.push(json!({"drawing": number, "description": description}));
        cursor = fence.body_end;
    }
    out.push_str(&markdown[cursor..]);
    (out, drawings)
}

// ----- reading a scene -----

fn num(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn is_full(element: &Value) -> bool {
    element.get("version").is_some_and(Value::is_number)
}

fn live(scene: &Value) -> Vec<&Value> {
    scene["elements"].as_array().map(|items| items.iter().filter(|e| e.get("isDeleted") != Some(&Value::Bool(true)) && str_of(e, "id").is_some()).collect()).unwrap_or_default()
}

fn is_shape(kind: &str) -> bool {
    matches!(kind, "rectangle" | "ellipse" | "diamond" | "image" | "frame" | "magicframe" | "embeddable" | "iframe")
}

fn is_linear(kind: &str) -> bool {
    matches!(kind, "arrow" | "line")
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > LABEL_CHARS {
        format!("{}…", flat.chars().take(LABEL_CHARS).collect::<String>())
    } else {
        flat
    }
}

#[derive(Debug, Clone, Copy)]
struct Box2 {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Box2 {
    fn of(element: &Value) -> Self {
        let (x, y, w, h) = (num(element, "x"), num(element, "y"), num(element, "width"), num(element, "height"));
        // Excalidraw allows a negative width or height on some elements.
        Box2 { x: x.min(x + w), y: y.min(y + h), w: w.abs(), h: h.abs() }
    }
    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    fn contains(&self, (px, py): (f64, f64), slack: f64) -> bool {
        px >= self.x - slack && px <= self.x + self.w + slack && py >= self.y - slack && py <= self.y + self.h + slack
    }
    fn area(&self) -> f64 {
        self.w * self.h
    }
}

/// A linear element's or a freehand stroke's points, in scene coordinates.
fn absolute_points(element: &Value) -> Vec<(f64, f64)> {
    let (x, y) = (num(element, "x"), num(element, "y"));
    element["points"]
        .as_array()
        .map(|points| points.iter().filter_map(|p| Some((x + p.get(0)?.as_f64()?, y + p.get(1)?.as_f64()?))).collect())
        .unwrap_or_default()
}

fn points_box(points: &[(f64, f64)]) -> Option<Box2> {
    let first = points.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for (x, y) in points {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    Some(Box2 { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// What a scene's elements say, indexed once.
struct Scene<'a> {
    elements: Vec<&'a Value>,
    by_id: HashMap<&'a str, &'a Value>,
    /// A container's bound text, by container id.
    bound_text: HashMap<&'a str, &'a str>,
}

impl<'a> Scene<'a> {
    fn new(scene: &'a Value) -> Self {
        let elements = live(scene);
        let by_id: HashMap<&str, &Value> = elements.iter().map(|e| (str_of(e, "id").unwrap(), *e)).collect();
        let mut bound_text = HashMap::new();
        for element in &elements {
            if str_of(element, "type") == Some("text") {
                if let Some(container) = str_of(element, "containerId").filter(|id| by_id.contains_key(id)) {
                    bound_text.insert(container, str_of(element, "originalText").or_else(|| str_of(element, "text")).unwrap_or(""));
                }
            }
        }
        Scene { elements, by_id, bound_text }
    }

    fn label(&self, element: &Value) -> Option<String> {
        let id = str_of(element, "id")?;
        let text = self
            .bound_text
            .get(id)
            .copied()
            .or_else(|| element.get("label").and_then(|label| str_of(label, "text")))
            .or_else(|| (str_of(element, "type") == Some("text")).then(|| str_of(element, "text")).flatten())
            .or_else(|| str_of(element, "name"))?;
        let text = one_line(text);
        (!text.is_empty()).then_some(text)
    }

    fn is_bound_text(&self, element: &Value) -> bool {
        str_of(element, "type") == Some("text") && str_of(element, "containerId").is_some_and(|id| self.by_id.contains_key(id))
    }

    fn name(&self, id: &str) -> String {
        match self.by_id.get(id) {
            Some(element) => match self.label(element) {
                Some(label) => format!("\"{label}\" [{id}]"),
                None => format!("{} [{id}]", str_of(element, "type").unwrap_or("element")),
            },
            None => format!("[{id}]"),
        }
    }

    fn shapes(&self) -> impl Iterator<Item = &&'a Value> {
        self.elements.iter().filter(|e| str_of(e, "type").is_some_and(|kind| is_shape(kind) && !kind.ends_with("frame")))
    }

    /// The smallest shape around a point, so a box inside a box is named.
    fn shape_at(&self, point: (f64, f64), slack: f64, skip: &str) -> Option<&'a Value> {
        self.shapes()
            .filter(|e| str_of(e, "id") != Some(skip) && Box2::of(e).contains(point, slack))
            .min_by(|a, b| Box2::of(a).area().total_cmp(&Box2::of(b).area()))
            .copied()
    }

    /// The element an arrow end is attached to: its binding, the skeleton's
    /// `start`/`end`, or the shape the end touches.
    fn end_of(&self, element: &Value, which: &str, point: Option<(f64, f64)>) -> Option<String> {
        let binding = if which == "start" { "startBinding" } else { "endBinding" };
        let bound = element.get(binding).and_then(|b| str_of(b, "elementId")).or_else(|| element.get(which).and_then(|end| str_of(end, "id")));
        if let Some(id) = bound.filter(|id| self.by_id.contains_key(id)) {
            return Some(id.to_owned());
        }
        point.and_then(|point| self.shape_at(point, 12.0, str_of(element, "id").unwrap_or(""))).and_then(|e| str_of(e, "id")).map(str::to_owned)
    }
}

/// Excalidraw's palette by name, both the stroke shades and the lighter fills
/// people pick for a background.
const PALETTE: [(&str, &str); 22] = [
    ("black", "#1e1e1e"),
    ("gray", "#868e96"),
    ("red", "#e03131"),
    ("pink", "#c2255c"),
    ("violet", "#6741d9"),
    ("blue", "#1971c2"),
    ("cyan", "#0c8599"),
    ("teal", "#099268"),
    ("green", "#2f9e44"),
    ("yellow", "#f08c00"),
    ("orange", "#e8590c"),
    ("light gray", "#e9ecef"),
    ("light red", "#ffc9c9"),
    ("light pink", "#fcc2d7"),
    ("light violet", "#d0bfff"),
    ("light blue", "#a5d8ff"),
    ("light cyan", "#99e9f2"),
    ("light teal", "#96f2d7"),
    ("light green", "#b2f2bb"),
    ("light yellow", "#ffec99"),
    ("light orange", "#ffd8a8"),
    ("white", "#ffffff"),
];

fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let hex = hex.strip_prefix('#')?;
    let hex = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect::<String>() } else { hex.get(..6)?.to_owned() };
    let value = u32::from_str_radix(&hex, 16).ok()?;
    Some((((value >> 16) & 255) as f64, ((value >> 8) & 255) as f64, (value & 255) as f64))
}

/// The nearest palette name for a color, so "#e03131" reads as "red".
fn color_name(color: &str) -> Option<String> {
    let lower = color.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "transparent" {
        return None;
    }
    if lower == "#000000" || lower == "#000" {
        return Some("black".into());
    }
    let (r, g, b) = rgb(&lower)?;
    PALETTE
        .iter()
        .filter_map(|(name, hex)| rgb(hex).map(|(pr, pg, pb)| (name, (r - pr).powi(2) + (g - pg).powi(2) + (b - pb).powi(2))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| (*name).to_owned())
}

/// A color an agent names, as Excalidraw stores it.
fn color_value(name: &str) -> Option<String> {
    let lower = name.trim().to_ascii_lowercase();
    if lower == "transparent" || lower == "none" {
        return Some("transparent".into());
    }
    if lower.starts_with('#') && rgb(&lower).is_some() {
        return Some(lower);
    }
    PALETTE.iter().find(|(key, _)| *key == lower).map(|(_, hex)| (*hex).to_owned())
}

fn style(element: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(stroke) = str_of(element, "strokeColor").and_then(color_name).filter(|name| name != "black") {
        parts.push(format!("{stroke} outline"));
    }
    if let Some(fill) = str_of(element, "backgroundColor").and_then(color_name).filter(|name| name != "white") {
        parts.push(format!("{fill} fill"));
    }
    match str_of(element, "strokeStyle") {
        Some("dashed") => parts.push("dashed".into()),
        Some("dotted") => parts.push("dotted".into()),
        _ => {}
    }
    if element.get("locked") == Some(&Value::Bool(true)) {
        parts.push("locked".into());
    }
    if let Some(link) = str_of(element, "link").filter(|link| !link.is_empty()) {
        parts.push(format!("links to {}", one_line(link)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(", {}", parts.join(", "))
    }
}

fn at(element: &Value) -> String {
    let b = Box2::of(element);
    format!("at ({}, {}) {}×{}", b.x.round(), b.y.round(), b.w.round(), b.h.round())
}

/// A drawing in plain lines an agent can reason about: every shape and text
/// with its id, label, place and color; every arrow by what it connects; every
/// freehand stroke by what it circles or marks. Reading order, top to bottom.
pub fn describe(scene: &Value) -> String {
    let scene_index = Scene::new(scene);
    let elements = &scene_index.elements;
    if elements.is_empty() {
        return "An empty drawing.".into();
    }
    let mut ordered: Vec<&&Value> = elements.iter().filter(|e| !scene_index.is_bound_text(e)).collect();
    ordered.sort_by(|a, b| {
        let (ab, bb) = (Box2::of(a), Box2::of(b));
        (ab.y.round() as i64, ab.x.round() as i64).cmp(&(bb.y.round() as i64, bb.x.round() as i64))
    });

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut items, mut links, mut marks, mut frames) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for element in &ordered {
        let id = str_of(element, "id").unwrap_or_default();
        let kind = str_of(element, "type").unwrap_or("element");
        match kind {
            "frame" | "magicframe" => {
                *counts.entry("frames").or_default() += 1;
                let members: Vec<String> = elements.iter().filter(|e| str_of(e, "frameId") == Some(id) && !scene_index.is_bound_text(e)).map(|e| scene_index.name(str_of(e, "id").unwrap_or(""))).collect();
                let label = scene_index.label(element).map(|label| format!(" \"{label}\"")).unwrap_or_default();
                frames.push(format!("- [{id}] frame{label} {}, holding {}", at(element), if members.is_empty() { "nothing".to_owned() } else { members.join(", ") }));
            }
            "text" => {
                *counts.entry("text").or_default() += 1;
                let inside = scene_index.shape_at(Box2::of(element).center(), 0.0, id).map(|shape| format!(", inside {}", scene_index.name(str_of(shape, "id").unwrap_or("")))).unwrap_or_default();
                items.push(format!("- [{id}] text \"{}\" {}{inside}{}", scene_index.label(element).unwrap_or_default(), at(element), style(element)));
            }
            "arrow" | "line" => {
                *counts.entry(if kind == "arrow" { "arrows" } else { "lines" }).or_default() += 1;
                let points = absolute_points(element);
                let from = scene_index.end_of(element, "start", points.first().copied());
                let to = scene_index.end_of(element, "end", points.last().copied());
                let place = |end: Option<String>, point: Option<(f64, f64)>| match (end, point) {
                    (Some(id), _) => scene_index.name(&id),
                    (None, Some((x, y))) => format!("({}, {})", x.round(), y.round()),
                    (None, None) => "?".into(),
                };
                let heads = (str_of(element, "startArrowhead").is_some(), kind == "arrow" && element.get("endArrowhead").map_or(true, |head| !head.is_null()));
                let verb = match heads {
                    (true, true) => "both ways between",
                    (true, false) => "back from",
                    (false, true) => "from",
                    (false, false) => "between",
                };
                let joiner = if verb == "from" || verb == "back from" { "to" } else { "and" };
                let label = scene_index.label(element).map(|label| format!(" \"{label}\"")).unwrap_or_default();
                let (first, last) = if verb == "back from" { (place(to, points.last().copied()), place(from, points.first().copied())) } else { (place(from, points.first().copied()), place(to, points.last().copied())) };
                links.push(format!("- [{id}] {kind}{label} {verb} {first} {joiner} {last}{}", style(element)));
            }
            "freedraw" => {
                *counts.entry("freehand").or_default() += 1;
                let points = absolute_points(element);
                let Some(bounds) = points_box(&points) else { continue };
                let closed = match (points.first(), points.last()) {
                    (Some(a), Some(b)) => ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() <= 0.25 * (bounds.w.powi(2) + bounds.h.powi(2)).sqrt().max(1.0),
                    _ => false,
                };
                let around: Vec<String> = scene_index
                    .shapes()
                    .copied()
                    .chain(ordered.iter().map(|e| **e).filter(|e| str_of(e, "type") == Some("text")))
                    .filter(|shape| bounds.contains(Box2::of(shape).center(), 4.0))
                    .map(|shape| scene_index.name(str_of(shape, "id").unwrap_or("")))
                    .collect::<Vec<_>>();
                let over = if around.is_empty() {
                    scene_index.shape_at(bounds.center(), 4.0, id).map(|shape| vec![scene_index.name(str_of(shape, "id").unwrap_or(""))]).unwrap_or_default()
                } else {
                    Vec::new()
                };
                let what = if closed && !around.is_empty() {
                    format!("loop drawn around {}", around.join(", "))
                } else if !around.is_empty() {
                    format!("stroke across {}", around.join(", "))
                } else if !over.is_empty() {
                    format!("mark on {}", over.join(", "))
                } else {
                    format!("stroke at ({}, {}) {}×{}", bounds.x.round(), bounds.y.round(), bounds.w.round(), bounds.h.round())
                };
                marks.push(format!("- [{id}] freehand {what}{}", style(element)));
            }
            _ => {
                *counts.entry(if is_shape(kind) { "shapes" } else { "other" }).or_default() += 1;
                let label = scene_index.label(element).map(|label| format!(" \"{label}\"")).unwrap_or_default();
                let inside = scene_index.shape_at(Box2::of(element).center(), 0.0, id).filter(|outer| Box2::of(outer).area() > Box2::of(element).area()).map(|shape| format!(", inside {}", scene_index.name(str_of(shape, "id").unwrap_or("")))).unwrap_or_default();
                let frame = str_of(element, "frameId").filter(|f| scene_index.by_id.contains_key(f)).map(|f| format!(", in frame {}", scene_index.name(f))).unwrap_or_default();
                items.push(format!("- [{id}] {kind}{label} {}{}{inside}{frame}", at(element), style(element)));
            }
        }
    }
    let summary = counts.iter().map(|(kind, count)| format!("{count} {kind}")).collect::<Vec<_>>().join(", ");
    let mut lines = vec![format!("{} elements: {summary}.", ordered.len())];
    for (title, group) in [("Shapes and text, top to bottom:", items), ("Connections:", links), ("Freehand marks:", marks), ("Frames:", frames)] {
        if !group.is_empty() {
            lines.push(title.into());
            lines.extend(group);
        }
    }
    if lines.len() > MAX_LINES {
        let more = lines.len() - MAX_LINES;
        lines.truncate(MAX_LINES);
        lines.push(format!("[clipped: {more} more lines]"));
    }
    lines.join("\n")
}

// ----- changing a scene -----

/// What one `notes_draw` call asks for.
#[derive(Debug, Default)]
pub struct Ops {
    pub add: Vec<Value>,
    pub update: Vec<Value>,
    pub remove: Vec<String>,
}

impl Ops {
    pub fn from_args(args: &Value) -> Result<Self, String> {
        let list = |key: &str| -> Result<Vec<Value>, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) if items.iter().all(Value::is_object) => Ok(items.clone()),
                Some(_) => Err(format!("{key} must be a list of objects")),
            }
        };
        let remove = match args.get("remove") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.iter().map(|item| item.as_str().map(str::to_owned).ok_or("remove must be a list of element ids")).collect::<Result<_, _>>()?,
            Some(_) => return Err("remove must be a list of element ids".into()),
        };
        let ops = Ops { add: list("add")?, update: list("update")?, remove };
        if ops.add.len() + ops.update.len() + ops.remove.len() > MAX_OPS {
            return Err(format!("one call changes at most {MAX_OPS} elements; split the drawing into several calls"));
        }
        Ok(ops)
    }

    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.update.is_empty() && self.remove.is_empty()
    }
}

/// What a change did, for the tool's answer.
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
}

fn new_id(taken: &HashSet<String>) -> String {
    loop {
        let id = format!("b{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        if !taken.contains(&id) {
            return id;
        }
    }
}

fn label_text(op: &Value) -> Option<String> {
    str_of(op, "label").or_else(|| str_of(op, "text")).map(str::to_owned)
}

/// A width that fits a label of this many characters at the default size.
fn fitted_width(label: Option<&str>) -> f64 {
    let longest = label.map(|text| text.lines().map(|line| line.chars().count()).max().unwrap_or(0)).unwrap_or(0) as f64;
    DEFAULT_WIDTH.max((longest * 11.0 + 40.0).min(420.0))
}

/// Style an agent asked for, in Excalidraw's fields.
fn apply_style(element: &mut Map<String, Value>, op: &Value) -> Result<(), String> {
    if let Some(color) = str_of(op, "color") {
        element.insert("strokeColor".into(), json!(color_value(color).ok_or_else(|| format!("unknown color {color}: use a hex value or {}", palette_names()))?));
    }
    if let Some(fill) = str_of(op, "fill") {
        let value = color_value(fill).ok_or_else(|| format!("unknown fill {fill}: use a hex value or {}", palette_names()))?;
        element.insert("fillStyle".into(), json!("solid"));
        element.insert("backgroundColor".into(), json!(value));
    }
    if let Some(dashed) = op.get("dashed").and_then(Value::as_bool) {
        element.insert("strokeStyle".into(), json!(if dashed { "dashed" } else { "solid" }));
    }
    Ok(())
}

fn palette_names() -> String {
    PALETTE.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
}

fn position(op: &Value) -> Option<(f64, f64)> {
    Some((op.get("x")?.as_f64()?, op.get("y")?.as_f64()?))
}

/// Where an arrow between two boxes runs: from the edge of one to the edge of
/// the other, along the line between their centers, a little short of both.
fn arrow_geometry(from: Box2, to: Box2) -> (f64, f64, f64, f64) {
    let (a, b) = (from.center(), to.center());
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let exit = |boxed: Box2, sign: f64| -> f64 {
        // How far along the center line the edge of `boxed` lies.
        let tx = if dx.abs() > f64::EPSILON { (boxed.w / 2.0) / dx.abs() } else { f64::INFINITY };
        let ty = if dy.abs() > f64::EPSILON { (boxed.h / 2.0) / dy.abs() } else { f64::INFINITY };
        let t = tx.min(ty);
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        sign * (t + ARROW_GAP / length)
    };
    if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
        return (a.0, a.1, 0.0, 0.0);
    }
    let start_t = exit(from, 1.0).min(0.45);
    let end_t = 1.0 + exit(to, -1.0).max(-0.45);
    let (sx, sy) = (a.0 + dx * start_t, a.1 + dy * start_t);
    let (ex, ey) = (a.0 + dx * end_t, a.1 + dy * end_t);
    (sx, sy, ex - sx, ey - sy)
}

fn route(arrow: &mut Map<String, Value>, from: Box2, to: Box2) {
    let (x, y, dx, dy) = arrow_geometry(from, to);
    arrow.insert("x".into(), json!(x.round()));
    arrow.insert("y".into(), json!(y.round()));
    arrow.insert("width".into(), json!(dx.abs().round()));
    arrow.insert("height".into(), json!(dy.abs().round()));
    arrow.insert("points".into(), json!([[0, 0], [dx.round(), dy.round()]]));
}

/// The fields an element keeps when it goes back to being a skeleton: the
/// editor completes it again on load, with a fresh label measured to fit.
fn demote(element: &Value, scene: &Scene) -> Map<String, Value> {
    let mut skeleton = element.as_object().cloned().unwrap_or_default();
    for key in ["version", "versionNonce", "seed", "updated", "index"] {
        skeleton.remove(key);
    }
    let id = str_of(element, "id").unwrap_or_default();
    if let Some(text) = scene.bound_text.get(id) {
        skeleton.insert("label".into(), json!({"text": text}));
    }
    // The label comes back from `label`; only arrows stay bound.
    if let Some(Value::Array(bound)) = skeleton.get_mut("boundElements") {
        bound.retain(|entry| str_of(entry, "type") == Some("arrow"));
    }
    if is_linear(str_of(element, "type").unwrap_or("")) {
        for (end, binding) in [("start", "startBinding"), ("end", "endBinding")] {
            if let Some(target) = element.get(binding).and_then(|b| str_of(b, "elementId")).filter(|target| scene.by_id.contains_key(target)) {
                skeleton.insert(end.into(), json!({"id": target}));
            }
            skeleton.remove(binding);
        }
    }
    skeleton
}

/// Apply `ops` to `scene`. Ids an agent passes are kept; any it leaves out
/// are made. Arrows name their ends by id and are routed between them.
/// Shapes added without a position are laid out left to right along the
/// arrows added with them, below what is already there.
pub fn apply(scene: &mut Value, ops: &Ops) -> Result<Applied, String> {
    let original = scene.clone();
    let index = Scene::new(&original);
    let mut taken: HashSet<String> = original["elements"].as_array().into_iter().flatten().filter_map(|e| str_of(e, "id")).map(str::to_owned).collect();
    let mut applied = Applied::default();

    // Removals first, with each removed shape's label and the arrows that hang
    // off it, so nothing is left pointing at a gap.
    let mut gone: HashSet<String> = HashSet::new();
    for id in &ops.remove {
        if !index.by_id.contains_key(id.as_str()) {
            return Err(format!("no element {id} in this drawing"));
        }
        gone.insert(id.clone());
    }
    let mut grew = true;
    while grew {
        grew = false;
        for element in &index.elements {
            let id = str_of(element, "id").unwrap_or_default();
            if gone.contains(id) {
                continue;
            }
            let container = str_of(element, "containerId").is_some_and(|c| gone.contains(c));
            let hanging = is_linear(str_of(element, "type").unwrap_or("")) && ["start", "end"].iter().any(|end| index.end_of(element, end, None).is_some_and(|target| gone.contains(&target)));
            if container || hanging {
                gone.insert(id.to_owned());
                grew = true;
            }
        }
    }
    let mut elements: Vec<Value> = original["elements"].as_array().cloned().unwrap_or_default().into_iter().filter(|e| !str_of(e, "id").is_some_and(|id| gone.contains(id))).collect();
    // What is left forgets what went: no binding or bound entry names it.
    for element in elements.iter_mut() {
        if let Some(Value::Array(bound)) = element.get_mut("boundElements") {
            bound.retain(|entry| !str_of(entry, "id").is_some_and(|id| gone.contains(id)));
        }
        for binding in ["startBinding", "endBinding"] {
            if element.get(binding).and_then(|b| str_of(b, "elementId")).is_some_and(|id| gone.contains(id)) {
                element[binding] = Value::Null;
            }
        }
    }
    let mut removed: Vec<String> = gone.into_iter().filter(|id| index.by_id.get(id.as_str()).is_some_and(|e| !index.is_bound_text(e))).collect();
    removed.sort();
    applied.removed = removed;

    // Updates: a changed label or text goes back to a skeleton, so the editor
    // measures it again; a moved or resized shape reroutes its arrows.
    let mut moved: HashSet<String> = HashSet::new();
    for op in &ops.update {
        let id = str_of(op, "id").ok_or("each update needs the id of an element")?.to_owned();
        let at = elements.iter().position(|e| str_of(e, "id") == Some(id.as_str())).ok_or_else(|| format!("no element {id} in this drawing"))?;
        let current = elements[at].clone();
        let kind = str_of(&current, "type").unwrap_or("").to_owned();
        if index.is_bound_text(&current) {
            return Err(format!("{id} is the label of {}; update that element's label instead", str_of(&current, "containerId").unwrap_or("its container")));
        }
        let relabel = label_text(op);
        let reshaped = ["x", "y", "width", "height"].iter().any(|key| op.get(*key).is_some_and(Value::is_number));
        // A full element whose label or box changes goes back to a skeleton,
        // so the editor measures and places its label again.
        let mut next = if (relabel.is_some() || reshaped) && is_full(&current) { demote(&current, &index) } else { current.as_object().cloned().unwrap_or_default() };
        if let Some(text) = relabel {
            if kind == "text" {
                next.insert("text".into(), json!(text));
                next.insert("originalText".into(), json!(text));
                next.remove("width");
                next.remove("height");
            } else if kind == "freedraw" {
                return Err(format!("{id} is a freehand stroke; it has no label"));
            } else {
                next.insert("label".into(), json!({"text": text}));
            }
        }
        for key in ["x", "y", "width", "height"] {
            if let Some(value) = op.get(key).and_then(Value::as_f64) {
                next.insert(key.into(), json!(value));
                if !is_linear(&kind) {
                    moved.insert(id.clone());
                }
            }
        }
        apply_style(&mut next, op)?;
        // A demoted container lost its bound text; drop the old one.
        if next.contains_key("label") && current.get("label").is_none() {
            elements.retain(|e| !(str_of(e, "type") == Some("text") && str_of(e, "containerId") == Some(id.as_str())));
        }
        let at = elements.iter().position(|e| str_of(e, "id") == Some(id.as_str())).unwrap();
        elements[at] = Value::Object(next);
        applied.updated.push(id);
    }

    // Additions: shapes and text first, so arrows can name them.
    let mut added_shapes: Vec<(String, Value)> = Vec::new();
    let mut added_links: Vec<Value> = Vec::new();
    for op in &ops.add {
        let kind = str_of(op, "type").unwrap_or("rectangle");
        if !matches!(kind, "rectangle" | "ellipse" | "diamond" | "text" | "arrow" | "line") {
            return Err(format!("an added element's type is rectangle, ellipse, diamond, text, arrow or line, not {kind}"));
        }
        let id = match str_of(op, "id") {
            Some(id) if taken.contains(id) => return Err(format!("{id} is already in this drawing; pick another id, or update it")),
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => new_id(&taken),
        };
        taken.insert(id.clone());
        let mut op = op.clone();
        op["id"] = json!(id);
        if is_linear(kind) {
            added_links.push(op);
        } else {
            added_shapes.push((id, op));
        }
    }

    // Lay out shapes that came without a position: a column per step along
    // the new arrows, below everything already drawn.
    let floor = elements.iter().filter(|e| !index.is_bound_text(e)).map(|e| {
        let b = Box2::of(e);
        b.y + b.h
    }).fold(f64::NEG_INFINITY, f64::max);
    let left = elements.iter().map(|e| Box2::of(e).x).fold(f64::INFINITY, f64::min);
    let (origin_x, origin_y) = (if left.is_finite() { left } else { 0.0 }, if floor.is_finite() { floor + ROW_GAP * 1.5 } else { 0.0 });
    let unplaced: Vec<&str> = added_shapes.iter().filter(|(_, op)| position(op).is_none()).map(|(id, _)| id.as_str()).collect();
    let mut depth: HashMap<&str, usize> = unplaced.iter().map(|id| (*id, 0)).collect();
    for _ in 0..unplaced.len() {
        for link in &added_links {
            if let (Some(from), Some(to)) = (str_of(link, "from"), str_of(link, "to")) {
                if let (Some(&d), true) = (depth.get(from), depth.contains_key(to)) {
                    if depth[to] < d + 1 && d + 1 < unplaced.len() {
                        depth.insert(to, d + 1);
                    }
                }
            }
        }
    }
    let mut row_in_column: HashMap<usize, usize> = HashMap::new();
    let mut boxes: HashMap<String, Box2> = elements.iter().filter_map(|e| Some((str_of(e, "id")?.to_owned(), Box2::of(e)))).collect();
    let mut new_shapes: Vec<Value> = Vec::new();
    for (id, op) in &added_shapes {
        let kind = str_of(op, "type").unwrap_or("rectangle");
        let label = label_text(op);
        let mut element = Map::new();
        element.insert("type".into(), json!(kind));
        element.insert("id".into(), json!(id));
        let (width, height) = if kind == "text" {
            (op.get("width").and_then(Value::as_f64), op.get("height").and_then(Value::as_f64))
        } else {
            (Some(op.get("width").and_then(Value::as_f64).unwrap_or_else(|| fitted_width(label.as_deref()))), Some(op.get("height").and_then(Value::as_f64).unwrap_or(DEFAULT_HEIGHT)))
        };
        let (x, y) = position(op).unwrap_or_else(|| {
            let column = depth.get(id.as_str()).copied().unwrap_or(0);
            let row = row_in_column.entry(column).or_default();
            let place = (origin_x + column as f64 * (DEFAULT_WIDTH + COLUMN_GAP + 40.0), origin_y + *row as f64 * (DEFAULT_HEIGHT + ROW_GAP));
            *row += 1;
            place
        });
        element.insert("x".into(), json!(x));
        element.insert("y".into(), json!(y));
        if let Some(width) = width {
            element.insert("width".into(), json!(width));
        }
        if let Some(height) = height {
            element.insert("height".into(), json!(height));
        }
        if kind == "text" {
            let text = label.clone().ok_or_else(|| format!("text {id} needs its text"))?;
            element.insert("text".into(), json!(text));
            element.insert("fontSize".into(), json!(op.get("fontSize").and_then(Value::as_f64).unwrap_or(20.0)));
            // A rough box until the editor measures it, so layout and the
            // description have something to go on.
            let lines = text.lines().count().max(1) as f64;
            let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
            boxes.insert(id.clone(), Box2 { x, y, w: width.unwrap_or(longest * 10.0), h: height.unwrap_or(lines * 25.0) });
        } else {
            if let Some(label) = &label {
                element.insert("label".into(), json!({"text": label}));
            }
            if kind != "diamond" {
                element.insert("roundness".into(), json!({"type": 3}));
            }
            boxes.insert(id.clone(), Box2 { x, y, w: width.unwrap_or(DEFAULT_WIDTH), h: height.unwrap_or(DEFAULT_HEIGHT) });
        }
        apply_style(&mut element, op)?;
        new_shapes.push(Value::Object(element));
        applied.added.push(id.clone());
    }

    // Arrows, routed between their ends' boxes, and any old arrow on a shape
    // that moved rerouted.
    let mut new_links: Vec<Value> = Vec::new();
    for op in &added_links {
        let kind = str_of(op, "type").unwrap_or("arrow");
        let id = str_of(op, "id").unwrap_or_default().to_owned();
        let (from, to) = (str_of(op, "from"), str_of(op, "to"));
        let (Some(from), Some(to)) = (from, to) else {
            return Err(format!("{kind} {id} needs from and to: the ids of the elements it joins"));
        };
        let from_box = *boxes.get(from).ok_or_else(|| format!("{kind} {id} starts at {from}, which is not in this drawing"))?;
        let to_box = *boxes.get(to).ok_or_else(|| format!("{kind} {id} ends at {to}, which is not in this drawing"))?;
        let mut element = Map::new();
        element.insert("type".into(), json!(kind));
        element.insert("id".into(), json!(id));
        route(&mut element, from_box, to_box);
        element.insert("start".into(), json!({"id": from}));
        element.insert("end".into(), json!({"id": to}));
        if kind == "arrow" {
            element.insert("endArrowhead".into(), json!("arrow"));
            if op.get("bothWays").and_then(Value::as_bool) == Some(true) {
                element.insert("startArrowhead".into(), json!("arrow"));
            }
        }
        if let Some(label) = label_text(op) {
            element.insert("label".into(), json!({"text": label}));
        }
        apply_style(&mut element, op)?;
        new_links.push(Value::Object(element));
        applied.added.push(id);
    }
    if !moved.is_empty() {
        for element in elements.iter_mut() {
            let kind = str_of(element, "type").unwrap_or("").to_owned();
            if !is_linear(&kind) {
                continue;
            }
            let ends = (index.end_of(element, "start", None), index.end_of(element, "end", None));
            let (Some(from), Some(to)) = ends else { continue };
            if !moved.contains(&from) && !moved.contains(&to) {
                continue;
            }
            let (Some(from_box), Some(to_box)) = (boxes.get(&from).copied(), boxes.get(&to).copied()) else { continue };
            let mut next = demote(element, &index);
            // Keep the label the editor bound to it.
            if let Some(text) = index.bound_text.get(str_of(element, "id").unwrap_or("")) {
                next.insert("label".into(), json!({"text": text}));
            }
            route(&mut next, from_box, to_box);
            next.insert("start".into(), json!({"id": from}));
            next.insert("end".into(), json!({"id": to}));
            *element = Value::Object(next);
        }
        // The rerouted arrows' old labels go; `label` brings them back.
        let relabeled: HashSet<String> = elements.iter().filter(|e| e.get("label").is_some() && !is_full(e)).filter_map(|e| str_of(e, "id").map(str::to_owned)).collect();
        elements.retain(|e| !(str_of(e, "type") == Some("text") && str_of(e, "containerId").is_some_and(|c| relabeled.contains(c))));
    }
    elements.extend(new_shapes);
    elements.extend(new_links);
    scene["elements"] = Value::Array(elements);
    Ok(applied)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A drawing as Excalidraw saves it after a person drew it: two boxes with
    /// bound labels, a bound arrow with its own label, a free text, a red
    /// box, and a freehand loop around one box.
    pub(crate) fn human_scene() -> Value {
        serde_json::from_str(include_str!("../../../../testing/fixtures/excalidraw/human-architecture.excalidraw")).unwrap()
    }

    fn fence(body: &str) -> String {
        format!("# Plan\n\nIntro.\n\n```excalidraw\n{body}\n```\n\nAfter.\n")
    }

    #[test]
    fn finds_drawing_fences_and_nothing_else() {
        let page = "```js\nlet a = 1;\n```\n\n````excalidraw\n{\"elements\":[]}\n````\n\n~~~ Excalidraw\n{}\n~~~\n```excalidraw\nunclosed";
        let fences = find(page);
        assert_eq!(fences.len(), 3);
        assert_eq!(&page[fences[0].body_start..fences[0].body_end], "{\"elements\":[]}\n");
        assert_eq!(&page[fences[1].body_start..fences[1].body_end], "{}\n");
        assert_eq!(&page[fences[2].body_start..fences[2].body_end], "unclosed");
        // A shorter closing run does not close a longer fence.
        assert_eq!(find("````excalidraw\n```\nstill in\n````\n")[0].body_end, "````excalidraw\n```\nstill in\n".len());
    }

    #[test]
    fn a_human_drawing_reads_as_labelled_connected_shapes() {
        let text = describe(&human_scene());
        assert!(text.contains("[api] rectangle \"API server\""), "{text}");
        assert!(text.contains("[db] rectangle \"Postgres\""), "{text}");
        // The arrow is named by what it joins, with its own label.
        assert!(text.contains("[a1] arrow \"SQL\" from \"API server\" [api] to \"Postgres\" [db]"), "{text}");
        // An unbound arrow is still read by where its ends land.
        assert!(text.contains("[a2] arrow from \"Web app\" [web] to \"API server\" [api]"), "{text}");
        assert!(text.contains("[legacy] rectangle \"Legacy cron\""), "{text}");
        assert!(text.contains("red outline"), "{text}");
        assert!(text.contains("[circle] freehand loop drawn around \"Postgres\" [db]"), "{text}");
        assert!(text.contains("[note] text \"remove this?\""), "{text}");
        // Bound labels are not listed twice, and deleted elements not at all.
        assert!(!text.contains("[api-label]"), "{text}");
        assert!(!text.contains("ghost"), "{text}");
    }

    #[test]
    fn read_swaps_bodies_for_markers_and_describes_each() {
        let page = fence(&serialize(&human_scene()));
        let (markdown, drawings) = for_agent(&page);
        assert!(markdown.contains("```excalidraw\n[drawing 1: see drawings[0]; change it with notes_draw, never notes_edit]\n```"), "{markdown}");
        assert!(markdown.starts_with("# Plan\n\nIntro.") && markdown.ends_with("After.\n"));
        assert!(!markdown.contains("\"elements\""));
        assert_eq!(drawings.len(), 1);
        assert!(drawings[0]["description"].as_str().unwrap().contains("Postgres"));
        let (broken, drawings) = for_agent(&fence("{not json"));
        assert!(broken.contains("[drawing 1:"));
        assert!(drawings[0]["description"].as_str().unwrap().contains("could not be read"));
    }

    #[test]
    fn serialize_puts_one_element_per_line_and_round_trips() {
        let scene = human_scene();
        let body = serialize(&scene);
        assert!(body.starts_with("{\"type\":\"excalidraw\",\"version\":2,\"source\":"), "{body}");
        assert_eq!(body.lines().count(), scene["elements"].as_array().unwrap().len() + 2);
        let back: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(back["elements"], scene["elements"]);
        assert_eq!(back["appState"], scene["appState"]);
    }

    #[test]
    fn an_agent_drawing_lays_out_shapes_and_routes_arrows_between_them() {
        let mut scene = empty_scene();
        let ops = Ops::from_args(&json!({"add": [
            {"id": "login", "label": "Login form"},
            {"id": "check", "type": "diamond", "label": "Valid?"},
            {"id": "home", "label": "Home", "fill": "light green"},
            {"id": "e1", "type": "arrow", "from": "login", "to": "check"},
            {"id": "e2", "type": "arrow", "from": "check", "to": "home", "label": "yes"},
        ]}))
        .unwrap();
        let applied = apply(&mut scene, &ops).unwrap();
        assert_eq!(applied.added, ["login", "check", "home", "e1", "e2"]);
        let elements = scene["elements"].as_array().unwrap();
        let get = |id: &str| elements.iter().find(|e| e["id"] == id).unwrap();
        // Laid out left to right along the arrows.
        assert!(num(get("login"), "x") < num(get("check"), "x") && num(get("check"), "x") < num(get("home"), "x"));
        // Skeletons, for the editor to complete.
        assert!(get("login").get("version").is_none());
        assert_eq!(get("e1")["start"]["id"], "login");
        assert_eq!(get("home")["backgroundColor"], "#b2f2bb");
        // The arrow starts at the first box's right edge and ends short of the next.
        let (lx, lw) = (num(get("login"), "x"), num(get("login"), "width"));
        assert!((num(get("e1"), "x") - (lx + lw)).abs() <= ARROW_GAP + 1.0, "{}", get("e1"));
        let end_x = num(get("e1"), "x") + get("e1")["points"][1][0].as_f64().unwrap();
        assert!(end_x < num(get("check"), "x"));
        let text = describe(&scene);
        assert!(text.contains("[e2] arrow \"yes\" from \"Valid?\" [check] to \"Home\" [home]"), "{text}");
        assert!(text.contains("light green fill"), "{text}");
    }

    #[test]
    fn an_agent_edit_keeps_what_the_person_drew() {
        let mut scene = human_scene();
        let before = scene["elements"].as_array().unwrap().clone();
        let ops = Ops::from_args(&json!({
            "add": [{"id": "cache", "label": "Redis cache", "x": 600, "y": 300}, {"id": "c1", "type": "arrow", "from": "api", "to": "cache"}],
            "update": [{"id": "db", "label": "Postgres 16", "color": "green"}],
            "remove": ["legacy"]
        }))
        .unwrap();
        let applied = apply(&mut scene, &ops).unwrap();
        assert_eq!(applied.removed, ["a3", "legacy"]);
        let elements = scene["elements"].as_array().unwrap();
        // The freehand loop and the untouched shapes are byte for byte as drawn.
        for id in ["circle", "web", "note", "a2"] {
            let was = before.iter().find(|e| e["id"] == id).unwrap();
            assert_eq!(elements.iter().find(|e| e["id"] == id), Some(was), "{id} changed");
        }
        // The relabelled box went back to a skeleton with the new label, and
        // its old bound text is gone.
        let db = elements.iter().find(|e| e["id"] == "db").unwrap();
        assert_eq!(db["label"]["text"], "Postgres 16");
        assert!(db.get("version").is_none());
        assert_eq!(db["strokeColor"], "#2f9e44");
        assert!(!elements.iter().any(|e| e["containerId"] == "db"));
        // The legacy box went with its label and the arrow that hung off it.
        assert!(!elements.iter().any(|e| e["id"] == "legacy" || e["containerId"] == "legacy" || e["id"] == "a3"));
        assert!(!elements.iter().any(|e| e["boundElements"].as_array().is_some_and(|bound| bound.iter().any(|b| b["id"] == "a3"))));
        let text = describe(&scene);
        assert!(text.contains("[a1] arrow \"SQL\" from \"API server\" [api] to \"Postgres 16\" [db]"), "{text}");
        assert!(text.contains("[c1] arrow from \"API server\" [api] to \"Redis cache\" [cache]"), "{text}");
    }

    #[test]
    fn moving_a_shape_reroutes_its_arrows_and_keeps_their_labels() {
        let mut scene = human_scene();
        apply(&mut scene, &Ops::from_args(&json!({"update": [{"id": "db", "x": 900, "y": 40}]})).unwrap()).unwrap();
        let elements = scene["elements"].as_array().unwrap();
        let a1 = elements.iter().find(|e| e["id"] == "a1").unwrap();
        assert_eq!(a1["end"]["id"], "db");
        assert_eq!(a1["label"]["text"], "SQL");
        assert!(num(a1, "x") + a1["points"][1][0].as_f64().unwrap() > 800.0, "{a1}");
        assert!(!elements.iter().any(|e| e["containerId"] == "a1"));
        assert!(describe(&scene).contains("[a1] arrow \"SQL\" from \"API server\" [api] to \"Postgres\" [db]"));
    }

    #[test]
    fn bad_requests_name_what_is_wrong() {
        let mut scene = human_scene();
        let err = |args: Value| apply(&mut scene.clone(), &Ops::from_args(&args).unwrap()).unwrap_err();
        assert!(err(json!({"remove": ["nope"]})).contains("no element nope"));
        assert!(err(json!({"add": [{"id": "api"}]})).contains("already in this drawing"));
        assert!(err(json!({"add": [{"type": "arrow", "from": "api"}]})).contains("needs from and to"));
        assert!(err(json!({"add": [{"type": "arrow", "from": "api", "to": "mars"}]})).contains("mars"));
        assert!(err(json!({"add": [{"label": "x", "fill": "plaid"}]})).contains("unknown fill plaid"));
        assert!(err(json!({"update": [{"id": "api-label", "label": "x"}]})).contains("label of api"));
        assert!(err(json!({"add": [{"type": "star"}]})).contains("not star"));
        assert!(Ops::from_args(&json!({"remove": [1]})).is_err());
        let too_many: Vec<Value> = (0..=MAX_OPS).map(|i| json!({"label": i.to_string()})).collect();
        assert!(Ops::from_args(&json!({"add": too_many})).is_err());
        // A refused change leaves the scene as it was.
        let before = scene.clone();
        assert!(apply(&mut scene, &Ops::from_args(&json!({"add": [{"label": "ok"}], "remove": ["nope"]})).unwrap()).is_err());
        assert_eq!(scene, before);
    }

    /// What `notes_draw` writes, checked in for the editor's tests to load:
    /// they complete these skeletons with Excalidraw itself.
    /// `BRIDGE_UPDATE_FIXTURES=1` rewrites them after a deliberate change.
    #[test]
    fn agent_fixtures_are_what_notes_draw_writes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/excalidraw");
        let mut login = empty_scene();
        apply(&mut login, &Ops::from_args(&json!({"add": [
            {"id": "form", "label": "Login form"},
            {"id": "check", "type": "diamond", "label": "Valid?"},
            {"id": "home", "label": "Home", "fill": "light green"},
            {"id": "retry", "type": "ellipse", "label": "Show error", "color": "red"},
            {"id": "hint", "type": "text", "label": "rate limited after 5 tries", "x": 0, "y": -80},
            {"id": "e1", "type": "arrow", "from": "form", "to": "check", "label": "submit"},
            {"id": "e2", "type": "arrow", "from": "check", "to": "home", "label": "yes"},
            {"id": "e3", "type": "arrow", "from": "check", "to": "retry", "label": "no", "dashed": true},
        ]})).unwrap()).unwrap();
        let mut mixed = human_scene();
        apply(&mut mixed, &Ops::from_args(&json!({
            "add": [{"id": "cache", "label": "Redis cache", "x": 640, "y": 220}, {"id": "c1", "type": "arrow", "from": "api", "to": "cache"}],
            "update": [{"id": "db", "label": "Postgres 16"}],
            "remove": ["legacy"]
        })).unwrap()).unwrap();
        for (name, scene) in [("agent-login.excalidraw", &login), ("human-plus-agent.excalidraw", &mixed)] {
            let path = dir.join(name);
            let body = serialize(scene) + "\n";
            if std::env::var_os("BRIDGE_UPDATE_FIXTURES").is_some() {
                std::fs::write(&path, &body).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), body, "{name} is stale: rerun with BRIDGE_UPDATE_FIXTURES=1");
        }
    }

    #[test]
    fn colors_read_as_names() {
        assert_eq!(color_name("#e03131").as_deref(), Some("red"));
        assert_eq!(color_name("#ffc9c9").as_deref(), Some("light red"));
        assert_eq!(color_name("#ff0000").as_deref(), Some("red"));
        assert_eq!(color_name("transparent"), None);
        assert_eq!(color_value("Light Blue").as_deref(), Some("#a5d8ff"));
        assert_eq!(color_value("#ABCDEF").as_deref(), Some("#abcdef"));
        assert_eq!(color_value("plaid"), None);
    }
}
