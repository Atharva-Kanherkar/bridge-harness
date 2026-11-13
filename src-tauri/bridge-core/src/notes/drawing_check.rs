//! What a person would see wrong in a drawing, said so an agent can fix it.
//!
//! `notes_draw` returns these problems after every change, so the agent checks
//! its own drawing before it answers: shapes on top of each other, an arrow's
//! label lying over a shape, an arrow cutting through a shape it does not join,
//! text over a shape, an arrow too short to show its head, crossing arrows,
//! and a drawing so large its text is unreadable once the page fits it.
//!
//! Sizes are the ones the editor will draw. An agent's shape grows to fit its
//! label the way Excalidraw grows it, and labels are measured with
//! Excalifont's widths ([`super::drawing_layout`]). Only problems an agent's
//! element is part of are reported: what the person drew is theirs.

use std::collections::HashMap;

use serde_json::Value;

use super::drawing::{absolute_points, is_full, live_elements, num, str_of, Box2};
use super::drawing_layout::{self as layout, LABEL_FONT};

/// Below this many pixels on the page, text is too small to read.
pub const MIN_TEXT_PX: f64 = 10.0;
const MAX_PROBLEMS: usize = 12;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub problems: Vec<String>,
    /// The drawing's size, and its smallest text once the page fits it.
    pub width: f64,
    pub height: f64,
    pub text_px: Option<f64>,
}

fn is_container(kind: &str) -> bool {
    matches!(kind, "rectangle" | "ellipse" | "diamond")
}

fn is_linear(kind: &str) -> bool {
    matches!(kind, "arrow" | "line")
}

fn font_size(element: &Value, default: f64) -> f64 {
    element.get("fontSize").and_then(Value::as_f64).filter(|size| *size > 0.0).unwrap_or(default)
}

/// A skeleton label's text and size, if it has one.
fn skeleton_label(element: &Value) -> Option<(&str, f64)> {
    let label = element.get("label")?;
    let text = str_of(label, "text").filter(|text| !text.trim().is_empty())?;
    let default = if is_linear(str_of(element, "type").unwrap_or("")) { 20.0 } else { LABEL_FONT };
    Some((text, font_size(label, default)))
}

/// The box an element takes once the editor draws it. An agent's container
/// grows to fit its label as Excalidraw grows it; an agent's text is measured.
pub fn rendered_box(element: &Value) -> Box2 {
    let kind = str_of(element, "type").unwrap_or("");
    let b = Box2::of(element);
    if is_full(element) {
        return b;
    }
    if kind == "text" {
        let text = str_of(element, "text").unwrap_or("");
        let size = font_size(element, LABEL_FONT);
        let (w, h) = layout::measure(text, f64::INFINITY, size);
        return Box2 { x: num(element, "x"), y: num(element, "y"), w, h };
    }
    if !is_container(kind) {
        return b;
    }
    let Some((text, size)) = skeleton_label(element) else { return b };
    let (max_w, max_h) = match kind {
        "ellipse" => ((b.w / 2.0 * std::f64::consts::SQRT_2).round() - 10.0, (b.h / 2.0 * std::f64::consts::SQRT_2).round() - 10.0),
        "diamond" => ((b.w / 2.0).round() - 10.0, (b.h / 2.0).round() - 10.0),
        _ => (b.w - 10.0, b.h - 10.0),
    };
    let (text_w, text_h) = layout::measure(text, max_w.max(1.0), size);
    let grow = |dimension: f64| match kind {
        "ellipse" => ((dimension.ceil() + 10.0) / std::f64::consts::SQRT_2 * 2.0).round(),
        "diamond" => 2.0 * (dimension.ceil() + 10.0),
        _ => dimension.ceil() + 10.0,
    };
    Box2 { x: b.x, y: b.y, w: if text_w > max_w { grow(text_w) } else { b.w }, h: if text_h > max_h { grow(text_h) } else { b.h } }
}

/// Where Excalidraw centers an arrow's label: its middle point, or the middle
/// of its middle segment.
fn label_point(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    match points.len() {
        0 => None,
        n if n % 2 == 1 => Some(points[n / 2]),
        n => {
            let (a, b) = (points[n / 2 - 1], points[n / 2]);
            Some(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0))
        }
    }
}

fn arrow_label_box(element: &Value, bound: Option<&Value>) -> Option<(Box2, String)> {
    if let Some(text) = bound {
        return Some((Box2::of(text), str_of(text, "originalText").or_else(|| str_of(text, "text")).unwrap_or("").to_owned()));
    }
    let (text, size) = skeleton_label(element)?;
    let points = absolute_points(element);
    let width = {
        let xs = points.iter().map(|p| p.0);
        xs.clone().fold(f64::NEG_INFINITY, f64::max) - xs.fold(f64::INFINITY, f64::min)
    };
    let (w, h) = layout::arrow_label_size(text, width.max(0.0), size);
    let (cx, cy) = label_point(&points)?;
    Some((Box2 { x: cx - w / 2.0, y: cy - h / 2.0, w, h }, text.to_owned()))
}

fn inflate(b: Box2, d: f64) -> Box2 {
    Box2 { x: b.x - d, y: b.y - d, w: (b.w + 2.0 * d).max(0.0), h: (b.h + 2.0 * d).max(0.0) }
}

pub fn overlaps(a: Box2, b: Box2) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn inside(inner: Box2, outer: Box2) -> bool {
    inner.x >= outer.x - 1.0 && inner.y >= outer.y - 1.0 && inner.x + inner.w <= outer.x + outer.w + 1.0 && inner.y + inner.h <= outer.y + outer.h + 1.0
}

fn cross((ox, oy): (f64, f64), (px, py): (f64, f64), (qx, qy): (f64, f64)) -> f64 {
    (px - ox) * (qy - oy) - (py - oy) * (qx - ox)
}

pub fn segments_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Whether the segment from `a` to `b` passes through the box.
pub fn segment_hits(a: (f64, f64), b: (f64, f64), r: Box2) -> bool {
    if r.contains(a, 0.0) || r.contains(b, 0.0) {
        return true;
    }
    let corners = [(r.x, r.y), (r.x + r.w, r.y), (r.x + r.w, r.y + r.h), (r.x, r.y + r.h)];
    (0..4).any(|i| segments_cross(a, b, corners[i], corners[(i + 1) % 4]))
}

/// A shape's outline, shrunk by `inset`: a diamond's and an ellipse's own,
/// so text beside a diamond's corner is not on the diamond.
fn outline(kind: &str, b: Box2, inset: f64) -> Vec<(f64, f64)> {
    let (cx, cy) = b.center();
    let (hw, hh) = ((b.w / 2.0 - inset).max(0.0), (b.h / 2.0 - inset).max(0.0));
    match kind {
        "diamond" => vec![(cx, cy - hh), (cx + hw, cy), (cx, cy + hh), (cx - hw, cy)],
        "ellipse" => (0..24).map(|i| f64::from(i) * std::f64::consts::PI / 12.0).map(|t| (cx + hw * t.cos(), cy + hh * t.sin())).collect(),
        _ => vec![(cx - hw, cy - hh), (cx + hw, cy - hh), (cx + hw, cy + hh), (cx - hw, cy + hh)],
    }
}

fn corners(b: Box2) -> Vec<(f64, f64)> {
    vec![(b.x, b.y), (b.x + b.w, b.y), (b.x + b.w, b.y + b.h), (b.x, b.y + b.h)]
}

/// Whether two convex polygons overlap (separating axis test).
fn polygons_overlap(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
    for poly in [a, b] {
        for i in 0..poly.len() {
            let ((x1, y1), (x2, y2)) = (poly[i], poly[(i + 1) % poly.len()]);
            let axis = (y1 - y2, x2 - x1);
            if axis.0 == 0.0 && axis.1 == 0.0 {
                continue;
            }
            let project = |p: &[(f64, f64)]| p.iter().map(|(x, y)| x * axis.0 + y * axis.1).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
            let ((a0, a1), (b0, b1)) = (project(a), project(b));
            if a1 <= b0 || b1 <= a0 {
                return false;
            }
        }
    }
    true
}

fn path_length(points: &[(f64, f64)]) -> f64 {
    points.windows(2).map(|pair| ((pair[1].0 - pair[0].0).powi(2) + (pair[1].1 - pair[0].1).powi(2)).sqrt()).sum()
}

/// Everything in `scene` a person would see wrong, involving an agent's element.
pub fn check(scene: &Value) -> Report {
    let elements = live_elements(scene);
    let by_id: HashMap<&str, &Value> = elements.iter().filter_map(|e| Some((str_of(e, "id")?, *e))).collect();
    let mut bound: HashMap<&str, &Value> = HashMap::new();
    for element in &elements {
        if str_of(element, "type") == Some("text") {
            if let Some(container) = str_of(element, "containerId").filter(|id| by_id.contains_key(id)) {
                bound.insert(container, element);
            }
        }
    }
    let label_of = |element: &Value| -> String {
        let id = str_of(element, "id").unwrap_or("");
        let text = bound
            .get(id)
            .and_then(|t| str_of(t, "originalText").or_else(|| str_of(t, "text")))
            .or_else(|| element.get("label").and_then(|l| str_of(l, "text")))
            .or_else(|| (str_of(element, "type") == Some("text")).then(|| str_of(element, "text")).flatten())
            .unwrap_or("");
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.is_empty() {
            format!("[{id}]")
        } else {
            let short: String = flat.chars().take(40).collect();
            format!("\"{short}{}\" [{id}]", if flat.chars().count() > 40 { "…" } else { "" })
        }
    };
    let agent = |element: &Value| !is_full(element);

    let shapes: Vec<(&Value, Box2)> = elements.iter().filter(|e| is_container(str_of(e, "type").unwrap_or("")) || matches!(str_of(e, "type"), Some("image" | "embeddable" | "iframe"))).map(|e| (*e, rendered_box(e))).collect();
    let texts: Vec<(&Value, Box2)> = elements.iter().filter(|e| str_of(e, "type") == Some("text") && !str_of(e, "containerId").is_some_and(|id| by_id.contains_key(id))).map(|e| (*e, rendered_box(e))).collect();
    let links: Vec<&Value> = elements.iter().copied().filter(|e| is_linear(str_of(e, "type").unwrap_or(""))).collect();
    let end_id = |link: &Value, which: &str| -> Option<String> {
        let binding = if which == "start" { "startBinding" } else { "endBinding" };
        link.get(binding).and_then(|b| str_of(b, "elementId")).or_else(|| link.get(which).and_then(|end| str_of(end, "id"))).filter(|id| by_id.contains_key(id)).map(str::to_owned)
    };

    let mut problems: Vec<String> = Vec::new();
    for (i, (a, ab)) in shapes.iter().enumerate() {
        for (b, bb) in shapes.iter().skip(i + 1) {
            if (agent(a) || agent(b)) && polygons_overlap(&outline(str_of(a, "type").unwrap_or(""), *ab, 2.0), &outline(str_of(b, "type").unwrap_or(""), *bb, 2.0)) && !inside(*ab, *bb) && !inside(*bb, *ab) {
                problems.push(format!("{} and {} overlap: move one, or leave out x and y and pass layout so the layout places them.", label_of(a), label_of(b)));
            }
        }
    }
    let mut label_boxes: Vec<(&Value, Box2, String)> = Vec::new();
    for link in &links {
        let id = str_of(link, "id").unwrap_or("");
        let Some((label_box, text)) = arrow_label_box(link, bound.get(id).copied()) else { continue };
        let ends = (end_id(link, "start"), end_id(link, "end"));
        for (shape, b) in &shapes {
            if !(agent(link) || agent(shape)) || !polygons_overlap(&corners(inflate(label_box, -1.0)), &outline(str_of(shape, "type").unwrap_or(""), *b, 2.0)) {
                continue;
            }
            let joined = [&ends.0, &ends.1].iter().any(|end| end.as_deref() == str_of(shape, "id"));
            let fix = if joined { "shorten the label, or leave more room between the shapes it joins (the layout does when you leave out x and y)" } else { "move the shape or the arrow" };
            problems.push(format!("the label \"{}\" on {} covers {}: {fix}.", text.split_whitespace().collect::<Vec<_>>().join(" "), id_ref(id), label_of(shape)));
        }
        label_boxes.push((*link, label_box, text));
    }
    for (text, tb) in &texts {
        for (shape, b) in &shapes {
            if (agent(text) || agent(shape)) && polygons_overlap(&corners(inflate(*tb, -1.0)), &outline(str_of(shape, "type").unwrap_or(""), *b, 2.0)) && !inside(*tb, *b) {
                problems.push(format!("the text {} lies across the edge of {}: move it inside or clear of it.", label_of(text), label_of(shape)));
            }
        }
    }
    let all_text: Vec<(&Value, Box2)> = texts.iter().map(|(e, b)| (*e, *b)).chain(label_boxes.iter().map(|(e, b, _)| (*e, *b))).collect();
    for (i, (a, ab)) in all_text.iter().enumerate() {
        for (b, bb) in all_text.iter().skip(i + 1) {
            if (agent(a) || agent(b)) && overlaps(inflate(*ab, -1.0), inflate(*bb, -1.0)) {
                problems.push(format!("the text of {} and of {} overlap: move one or shorten it.", label_of(a), label_of(b)));
            }
        }
    }
    let mut crossings: Vec<String> = Vec::new();
    for (i, link) in links.iter().enumerate() {
        let id = str_of(link, "id").unwrap_or("");
        let points = absolute_points(link);
        if points.len() < 2 {
            continue;
        }
        let (start, end) = (end_id(link, "start"), end_id(link, "end"));
        if agent(link) && str_of(link, "type") == Some("arrow") && path_length(&points) < 24.0 {
            problems.push(format!("the arrow {} is too short to show its head: move its ends apart.", id_ref(id)));
        }
        for (shape, b) in &shapes {
            let shape_id = str_of(shape, "id");
            if shape_id == start.as_deref() || shape_id == end.as_deref() || !(agent(link) || agent(shape)) {
                continue;
            }
            // A shape that holds either end is a group around the arrow.
            if b.contains(points[0], 0.0) || b.contains(*points.last().unwrap(), 0.0) {
                continue;
            }
            let inner = outline(str_of(shape, "type").unwrap_or(""), *b, 4.0);
            if points.windows(2).any(|pair| polygons_overlap(&[pair[0], pair[1]], &inner)) {
                problems.push(format!("{} {} cuts through {}, which it does not join: move {} or pass layout.", str_of(link, "type").unwrap_or("arrow"), id_ref(id), label_of(shape), id_ref(shape_id.unwrap_or(""))));
            }
        }
        for other in links.iter().skip(i + 1) {
            if !(agent(link) || agent(other)) {
                continue;
            }
            let theirs = absolute_points(other);
            if points.windows(2).any(|a| theirs.windows(2).any(|b| segments_cross(a[0], a[1], b[0], b[1]))) {
                crossings.push(format!("{} crosses {}", id_ref(id), id_ref(str_of(other, "id").unwrap_or(""))));
            }
        }
    }
    if !crossings.is_empty() {
        let shown: Vec<String> = crossings.iter().take(4).cloned().collect();
        problems.push(format!("{}{}: reorder the shapes, or pass layout to place them with fewer crossings.", shown.join(", "), if crossings.len() > 4 { format!(" and {} more crossings", crossings.len() - 4) } else { String::new() }));
    }

    // How it reads on the page.
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut grow = |b: Box2| {
        x0 = x0.min(b.x);
        y0 = y0.min(b.y);
        x1 = x1.max(b.x + b.w);
        y1 = y1.max(b.y + b.h);
    };
    for element in &elements {
        let kind = str_of(element, "type").unwrap_or("");
        if is_linear(kind) || kind == "freedraw" {
            for point in absolute_points(element) {
                grow(Box2 { x: point.0, y: point.1, w: 0.0, h: 0.0 });
            }
        } else if !(kind == "text" && str_of(element, "containerId").is_some_and(|id| by_id.contains_key(id))) {
            grow(rendered_box(element));
        }
    }
    for (_, b, _) in &label_boxes {
        grow(*b);
    }
    let mut report = Report::default();
    if x0.is_finite() {
        report.width = (x1 - x0).round();
        report.height = (y1 - y0).round();
        let scale = layout::page_scale(report.width, report.height);
        let smallest = elements
            .iter()
            .filter_map(|e| match str_of(e, "type") {
                Some("text") => Some(font_size(e, LABEL_FONT)),
                Some(kind) if is_linear(kind) => skeleton_label(e).map(|(_, size)| size),
                Some(kind) if is_container(kind) => skeleton_label(e).map(|(_, size)| size),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min);
        if smallest.is_finite() {
            let px = (smallest * scale * 10.0).round() / 10.0;
            report.text_px = Some(px);
            if px < MIN_TEXT_PX && elements.iter().any(|e| agent(e)) {
                problems.push(format!(
                    "the drawing is {}×{} px, so once the page fits it its smallest text shows at {px} px, too small to read: shorten labels, pass layout so it winds to fit the page, or split it into two drawings.",
                    report.width, report.height
                ));
            }
        }
    }
    if problems.len() > MAX_PROBLEMS {
        let more = problems.len() - MAX_PROBLEMS;
        problems.truncate(MAX_PROBLEMS);
        problems.push(format!("and {more} more problems like these"));
    }
    report.problems = problems;
    report
}

fn id_ref(id: &str) -> String {
    format!("[{id}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scene(elements: Value) -> Value {
        json!({"type": "excalidraw", "elements": elements})
    }

    #[test]
    fn an_agent_drawing_with_room_has_no_problems() {
        let report = check(&scene(json!([
            {"id": "a", "type": "rectangle", "x": 0, "y": 0, "width": 140, "height": 60, "label": {"text": "Login form"}},
            {"id": "b", "type": "diamond", "x": 260, "y": -15, "width": 180, "height": 90, "label": {"text": "Valid?"}},
            {"id": "e", "type": "arrow", "x": 146, "y": 30, "width": 108, "height": 0, "points": [[0, 0], [108, 0]], "start": {"id": "a"}, "end": {"id": "b"}, "label": {"text": "submit", "fontSize": 16}},
        ])));
        assert_eq!(report.problems, Vec::<String>::new());
        assert!(report.text_px.unwrap() >= 10.0, "{report:?}");
    }

    #[test]
    fn what_a_person_would_see_wrong_is_named_with_the_fix() {
        let report = check(&scene(json!([
            {"id": "a", "type": "rectangle", "x": 0, "y": 0, "width": 140, "height": 60, "label": {"text": "Orders service"}},
            {"id": "b", "type": "rectangle", "x": 200, "y": 0, "width": 140, "height": 60, "label": {"text": "Kafka"}},
            {"id": "c", "type": "rectangle", "x": 120, "y": 30, "width": 140, "height": 60, "label": {"text": "Overlapping"}},
            {"id": "far", "type": "rectangle", "x": 700, "y": 0, "width": 140, "height": 60, "label": {"text": "Far"}},
            {"id": "e", "type": "arrow", "x": 146, "y": 10, "width": 48, "height": 0, "points": [[0, 0], [48, 0]], "start": {"id": "a"}, "end": {"id": "b"}, "label": {"text": "publishes events to the topic"}},
            {"id": "f", "type": "arrow", "x": 0, "y": 120, "width": 840, "height": 0, "points": [[0, 0], [10, -100], [840, -90]], "start": {"id": "a"}, "end": {"id": "far"}},
        ])));
        let all = report.problems.join("\n");
        assert!(all.contains("\"Orders service\" [a] and \"Overlapping\" [c] overlap"), "{all}");
        assert!(all.contains("the label \"publishes events to the topic\" on [e] covers \"Orders service\" [a]"), "{all}");
        assert!(all.contains("[f] cuts through"), "{all}");
    }

    #[test]
    fn a_label_beside_a_diamond_corner_is_not_on_it_and_a_person_drawing_is_left_alone() {
        let beside = check(&scene(json!([
            {"id": "q", "type": "diamond", "x": 0, "y": 0, "width": 200, "height": 100, "label": {"text": "Ok?"}},
            {"id": "t", "type": "text", "x": 4, "y": 4, "text": "yes", "fontSize": 16},
        ])));
        assert_eq!(beside.problems, Vec::<String>::new());
        let person = check(&scene(json!([
            {"id": "a", "type": "rectangle", "x": 0, "y": 0, "width": 140, "height": 60, "version": 3},
            {"id": "b", "type": "rectangle", "x": 50, "y": 20, "width": 140, "height": 60, "version": 3},
        ])));
        assert_eq!(person.problems, Vec::<String>::new());
    }

    #[test]
    fn a_drawing_too_large_for_the_page_says_so() {
        let wide: Vec<Value> = (0..12).map(|i| json!({"id": format!("n{i}"), "type": "rectangle", "x": i * 260, "y": 0, "width": 200, "height": 60, "label": {"text": "Step"}})).collect();
        let report = check(&scene(Value::Array(wide)));
        assert!(report.text_px.unwrap() < MIN_TEXT_PX);
        assert!(report.problems.iter().any(|p| p.contains("too small to read")), "{:?}", report.problems);
    }
}
