//! Where an agent's shapes go and how big they are, so a drawing reads at a
//! glance once the page shows it.
//!
//! Every shape is sized to its label as Excalidraw will wrap it, measured with
//! Excalifont's own widths. Shapes joined by arrows are laid out in layers
//! (the Sugiyama method: ranks along the arrows, an order in each rank with
//! few crossings, each shape pulled level with its neighbours), with the gap
//! between ranks wide enough for the arrow labels in it. A long chain winds
//! back and forth in rows instead of running off the page. The direction,
//! across or down, is the one that shows the text largest once the drawing
//! fits the note column, unless the agent names one.

use std::collections::{HashMap, HashSet};

/// Excalifont's advance widths at 20 px for ASCII 32..=126, measured with the
/// editor's canvas (`scripts/drawing-bench`). They add up exactly: Excalifont
/// has no kerning, so a line's width is the sum of its characters'.
const EXCALIFONT_20: [f64; 95] = [
    8.0, 6.28, 7.42, 15.66, 14.42, 18.56, 14.36, 4.36, 8.82, 8.04, 10.5, 11.0, 5.14, 8.22, 5.48, 11.22, 13.28, 8.54, 14.0, 12.16, 11.7, 12.36, 12.8, 11.16, 12.72, 12.58, 5.28, 5.96, 11.0, 11.0, 11.0, 9.32, 16.58, 13.52,
    15.22, 12.58, 15.6, 14.14, 13.22, 15.6, 11.46, 10.9, 11.38, 12.26, 10.86, 15.32, 12.64, 15.34, 13.96, 15.36, 14.72, 12.44, 17.14, 14.6, 11.84, 15.72, 12.56, 11.28, 16.64, 9.44, 11.78, 9.94, 10.2, 13.4, 12.0, 11.52, 11.1,
    10.08, 12.1, 10.74, 9.94, 11.1, 11.34, 4.88, 6.56, 10.66, 4.5, 13.26, 10.52, 12.0, 10.74, 10.78, 8.24, 10.86, 11.06, 10.96, 10.5, 13.86, 11.82, 10.6, 11.44, 10.08, 5.98, 10.88, 13.38,
];

pub const LINE_HEIGHT: f64 = 1.25;
pub const LABEL_FONT: f64 = 20.0;
/// Arrow labels are a size smaller than shape labels, so they read as notes on
/// the arrow rather than as another box.
pub const ARROW_LABEL_FONT: f64 = 16.0;
/// Excalidraw's room between a container's edge and its label.
const BOUND_PADDING: f64 = 5.0;
/// The note column a drawing preview fills and its height cap
/// (`DrawingPreview`: `w-full max-h-[28rem]`, export padding 16).
pub const PAGE_WIDTH: f64 = 640.0;
pub const PAGE_HEIGHT: f64 = 448.0;
pub const PAGE_PADDING: f64 = 16.0;
pub const MIN_WIDTH: f64 = 100.0;
pub const MIN_HEIGHT: f64 = 56.0;
/// Between ranks when no arrow label needs more, and between neighbours in a rank.
pub const RANK_GAP: f64 = 60.0;
pub const NODE_GAP: f64 = 30.0;
/// Clear space on each side of an arrow label.
pub const LABEL_MARGIN: f64 = 16.0;
/// Below this text size on the page (scale 0.6 of a 20 px label), a layout
/// that reads better in the other direction wins.
const COMFORTABLE_SCALE: f64 = 0.6;

fn is_wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1FAFF | 0x20000..=0x3FFFD)
}

pub fn char_width(c: char, size: f64) -> f64 {
    let at_20 = match c as u32 {
        32..=126 => EXCALIFONT_20[(c as u32 - 32) as usize],
        _ if is_wide(c) => 20.0,
        _ => 12.0,
    };
    at_20 * size / 20.0
}

pub fn text_width(line: &str, size: f64) -> f64 {
    line.chars().map(|c| char_width(c, size)).sum()
}

/// Text wrapped the way Excalidraw wraps a label: words fill a line up to
/// `max`, and a word longer than a line breaks between characters. Line
/// breaks the author wrote are kept.
pub fn wrap(text: &str, max: f64, size: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ').filter(|word| !word.is_empty()) {
            let joined = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if text_width(&joined, size) <= max {
                line = joined;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for c in word.chars() {
                if !line.is_empty() && text_width(&line, size) + char_width(c, size) > max {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    lines
}

/// The box a text takes when wrapped at `max`.
pub fn measure(text: &str, max: f64, size: f64) -> (f64, f64) {
    let lines = wrap(text, max, size);
    let width = lines.iter().map(|line| text_width(line, size)).fold(0.0, f64::max);
    (width, lines.len().max(1) as f64 * size * LINE_HEIGHT)
}

/// The size a shape needs for Excalidraw to draw its label inside it without
/// growing it: a rectangle's label may use its width less the padding, an
/// ellipse's the square inside it, a diamond's half its width.
pub fn fit_shape(kind: &str, label: Option<&str>) -> (f64, f64) {
    let Some(label) = label.filter(|label| !label.trim().is_empty()) else {
        return (MIN_WIDTH, MIN_HEIGHT);
    };
    let (wrap_at, pad_x, pad_y) = match kind {
        "ellipse" => (170.0, 8.0, 6.0),
        "diamond" => (150.0, 8.0, 4.0),
        _ => (240.0, 18.0, 12.0),
    };
    let (text_w, text_h) = measure(label, wrap_at, LABEL_FONT);
    let inner_w = text_w + 2.0 * (BOUND_PADDING + pad_x);
    let inner_h = text_h + 2.0 * (BOUND_PADDING + pad_y);
    let (w, h) = match kind {
        "ellipse" => (inner_w * std::f64::consts::SQRT_2, inner_h * std::f64::consts::SQRT_2),
        "diamond" => (inner_w * 2.0, inner_h * 2.0),
        _ => (inner_w, inner_h),
    };
    (w.max(MIN_WIDTH).ceil(), h.max(MIN_HEIGHT).ceil())
}

/// The box Excalidraw gives an arrow's label: wrapped at 70% of the arrow's
/// width or eleven times the font size, whichever is more.
pub fn arrow_label_size(text: &str, arrow_width: f64, size: f64) -> (f64, f64) {
    measure(text, (0.7 * arrow_width).max(11.0 * size), size)
}

/// How large a drawing of this size shows once the preview fits it to the page.
pub fn page_scale(width: f64, height: f64) -> f64 {
    (PAGE_WIDTH / (width + 2.0 * PAGE_PADDING)).min(PAGE_HEIGHT / (height + 2.0 * PAGE_PADDING))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Right,
    Down,
}

impl Direction {
    pub fn parse(value: &str) -> Option<Option<Self>> {
        match value {
            "auto" => Some(None),
            "right" | "across" => Some(Some(Direction::Right)),
            "down" => Some(Some(Direction::Down)),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Direction::Right => "right",
            Direction::Down => "down",
        }
    }
}

/// Shapes to place, by index, and the arrows between them with their label
/// boxes (already measured).
#[derive(Debug, Default, Clone)]
pub struct Graph {
    pub sizes: Vec<(f64, f64)>,
    pub edges: Vec<(usize, usize, Option<(f64, f64)>)>,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub direction: Direction,
    /// Each shape's top-left corner.
    pub positions: Vec<(f64, f64)>,
    /// Each arrow's bend points, from its start to its end.
    pub bends: Vec<Vec<(f64, f64)>>,
    pub width: f64,
    pub height: f64,
}

impl Layout {
    pub fn scale(&self) -> f64 {
        page_scale(self.width, self.height)
    }
}

/// The layout that reads best: the direction asked for, or, without one,
/// whichever shows the text larger on the page. A tree reads down and
/// anything else across, unless the other way is clearly easier to read.
pub fn arrange(graph: &Graph, direction: Option<Direction>) -> Layout {
    if let Some(direction) = direction {
        return best_of(graph, direction);
    }
    let tree = graph.edges.iter().any(|(from, _, _)| graph.edges.iter().filter(|(other, _, _)| other == from).count() >= 2)
        && (0..graph.sizes.len()).all(|node| graph.edges.iter().filter(|(_, to, _)| *to == node).count() <= 1);
    let (preferred, other) = if tree { (Direction::Down, Direction::Right) } else { (Direction::Right, Direction::Down) };
    let (first, second) = (best_of(graph, preferred), best_of(graph, other));
    if first.scale() >= COMFORTABLE_SCALE || first.scale() >= 0.92 * second.scale() {
        first
    } else {
        second
    }
}

/// A layered layout in `direction`, or for a plain chain a winding one when
/// that reads larger.
fn best_of(graph: &Graph, direction: Direction) -> Layout {
    let layered = layered(graph, direction);
    match chain_order(graph) {
        Some(order) if order.len() >= 4 => {
            let wound = (2..=order.len().div_ceil(2)).map(|per_row| winding(graph, &order, per_row, direction)).max_by(|a, b| a.scale().total_cmp(&b.scale()));
            match wound {
                Some(wound) if wound.scale() > layered.scale() * 1.15 => wound,
                _ => layered,
            }
        }
        _ => layered,
    }
}

/// The nodes in order when the graph is one simple path.
fn chain_order(graph: &Graph) -> Option<Vec<usize>> {
    let n = graph.sizes.len();
    if n < 2 || graph.edges.len() != n - 1 {
        return None;
    }
    let mut next = vec![None; n];
    let mut has_prev = vec![false; n];
    for (from, to, _) in &graph.edges {
        if from == to || next[*from].is_some() || has_prev[*to] {
            return None;
        }
        next[*from] = Some(*to);
        has_prev[*to] = true;
    }
    let start = (0..n).find(|node| !has_prev[*node])?;
    let mut order = vec![start];
    while let Some(node) = next[*order.last().unwrap()] {
        order.push(node);
        if order.len() > n {
            return None;
        }
    }
    (order.len() == n).then_some(order)
}

/// A chain laid out in rows of `per_row` that run alternately forward and back
/// (columns, for `Down`), so it fills the page instead of one long line.
fn winding(graph: &Graph, order: &[usize], per_row: usize, direction: Direction) -> Layout {
    let label_of = |from: usize, to: usize| graph.edges.iter().find(|(a, b, _)| *a == from && *b == to).and_then(|(_, _, label)| *label);
    // Swap axes for `Down`: work in (along, across) and turn at the end.
    let along_size = |node: usize| if direction == Direction::Right { graph.sizes[node] } else { (graph.sizes[node].1, graph.sizes[node].0) };
    let rows: Vec<&[usize]> = order.chunks(per_row).collect();
    let cell = rows.iter().flat_map(|row| row.iter()).map(|node| along_size(*node).0).fold(0.0, f64::max);
    let mut gap_along = RANK_GAP;
    let mut row_gaps = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        for pair in row.windows(2) {
            if let Some(label) = label_of(pair[0], pair[1]) {
                let need = if direction == Direction::Right { label.0 } else { label.1 };
                gap_along = gap_along.max(need + 2.0 * LABEL_MARGIN);
            }
        }
        if let Some(next) = rows.get(index + 1) {
            let turn = label_of(*row.last().unwrap(), next[0]).map(|label| if direction == Direction::Right { label.1 } else { label.0 } + 2.0 * LABEL_MARGIN).unwrap_or(0.0);
            row_gaps.push(RANK_GAP.max(turn));
        }
    }
    let mut positions = vec![(0.0, 0.0); graph.sizes.len()];
    let mut across = 0.0;
    for (index, row) in rows.iter().enumerate() {
        let thickness = row.iter().map(|node| along_size(*node).1).fold(0.0, f64::max);
        for (slot, node) in row.iter().enumerate() {
            let column = if index % 2 == 0 { slot } else { per_row - 1 - slot };
            let (a, b) = along_size(*node);
            let along = column as f64 * (cell + gap_along) + (cell - a) / 2.0;
            let offset = across + (thickness - b) / 2.0;
            positions[*node] = if direction == Direction::Right { (along, offset) } else { (offset, along) };
        }
        across += thickness + row_gaps.get(index).copied().unwrap_or(0.0);
    }
    finish(graph, direction, positions, vec![Vec::new(); graph.edges.len()])
}

/// Shift everything to start at the origin and measure it.
fn finish(graph: &Graph, direction: Direction, mut positions: Vec<(f64, f64)>, mut bends: Vec<Vec<(f64, f64)>>) -> Layout {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (index, (x, y)) in positions.iter().enumerate() {
        let (w, h) = graph.sizes[index];
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(x + w);
        y1 = y1.max(y + h);
    }
    for (x, y) in bends.iter().flatten() {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    if !x0.is_finite() {
        return Layout { direction, positions, bends, width: 0.0, height: 0.0 };
    }
    for (x, y) in positions.iter_mut() {
        *x = (*x - x0).round();
        *y = (*y - y0).round();
    }
    for (x, y) in bends.iter_mut().flatten() {
        *x = (*x - x0).round();
        *y = (*y - y0).round();
    }
    Layout { direction, positions, bends, width: (x1 - x0).round(), height: (y1 - y0).round() }
}

/// One place in a rank: a shape, or a bend of an arrow that spans ranks.
#[derive(Debug, Clone)]
struct Slot {
    node: Option<usize>,
    /// Size along the ranks and across them.
    along: f64,
    across: f64,
}

/// The layered (Sugiyama) layout.
pub fn layered(graph: &Graph, direction: Direction) -> Layout {
    let n = graph.sizes.len();
    let edge_count = graph.edges.len();
    // 1. Point every arrow forward: a depth-first walk reverses those that close a cycle.
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (index, (from, to, _)) in graph.edges.iter().enumerate() {
        if from != to {
            out[*from].push(index);
        }
    }
    let mut reversed = vec![false; edge_count];
    let mut state = vec![0u8; n];
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        state[root] = 1;
        while let Some((node, next)) = stack.last_mut() {
            let node = *node;
            if let Some(&edge) = out[node].get(*next) {
                *next += 1;
                let to = graph.edges[edge].1;
                match state[to] {
                    0 => {
                        state[to] = 1;
                        stack.push((to, 0));
                    }
                    1 => reversed[edge] = true,
                    _ => {}
                }
            } else {
                state[node] = 2;
                stack.pop();
            }
        }
    }
    let oriented: Vec<Option<(usize, usize)>> = graph
        .edges
        .iter()
        .enumerate()
        .map(|(index, (from, to, _))| (from != to).then(|| if reversed[index] { (*to, *from) } else { (*from, *to) }))
        .collect();

    // 2. Ranks: the longest path to each shape; a shape that only points out
    // sits just before the nearest shape it points to.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, to) in oriented.iter().flatten() {
        if !succs[*from].contains(to) {
            succs[*from].push(*to);
            preds[*to].push(*from);
        }
    }
    let mut indegree: Vec<usize> = preds.iter().map(Vec::len).collect();
    let mut queue: Vec<usize> = (0..n).filter(|node| indegree[*node] == 0).collect();
    let mut topo = Vec::with_capacity(n);
    let mut cursor = 0;
    while cursor < queue.len() {
        let node = queue[cursor];
        cursor += 1;
        topo.push(node);
        for succ in &succs[node] {
            indegree[*succ] -= 1;
            if indegree[*succ] == 0 {
                queue.push(*succ);
            }
        }
    }
    let mut rank = vec![0usize; n];
    for node in &topo {
        for succ in &succs[*node] {
            rank[*succ] = rank[*succ].max(rank[*node] + 1);
        }
    }
    for node in topo.iter().rev() {
        if preds[*node].is_empty() {
            if let Some(nearest) = succs[*node].iter().map(|succ| rank[*succ]).min() {
                rank[*node] = nearest.saturating_sub(1).max(rank[*node]);
            }
        }
    }
    let ranks = rank.iter().copied().max().map_or(0, |top| top + 1);

    // 3. Slots, with a bend slot wherever an arrow crosses a rank.
    let size_along = |(w, h): (f64, f64)| if direction == Direction::Right { w } else { h };
    let size_across = |(w, h): (f64, f64)| if direction == Direction::Right { h } else { w };
    let mut slots: Vec<Slot> = (0..n).map(|node| Slot { node: Some(node), along: size_along(graph.sizes[node]), across: size_across(graph.sizes[node]) }).collect();
    let mut layer_of: Vec<usize> = rank.clone();
    let mut chains: Vec<Vec<usize>> = vec![Vec::new(); edge_count];
    // Each arrow's label: where it sits, and the room the gap it sits in needs.
    let mut gap_need: Vec<f64> = vec![RANK_GAP; ranks.max(1)];
    let mut segments: Vec<(usize, usize)> = Vec::new();
    for (index, pair) in oriented.iter().enumerate() {
        let Some((from, to)) = *pair else { continue };
        let label = graph.edges[index].2;
        let span = rank[to] - rank[from];
        let mut previous = from;
        for step in 1..span {
            let middle = (span - 1) % 2 == 1 && step - 1 == (span - 1) / 2;
            let (along, across) = match (label, middle) {
                (Some(label), true) => (size_along(label) + 2.0 * LABEL_MARGIN, size_across(label)),
                _ => (0.0, 8.0),
            };
            slots.push(Slot { node: None, along, across });
            layer_of.push(rank[from] + step);
            let slot = slots.len() - 1;
            chains[index].push(slot);
            segments.push((previous, slot));
            previous = slot;
        }
        segments.push((previous, to));
        // With an even number of points the label sits mid-segment, in a gap.
        if let Some(label) = label {
            if (span - 1) % 2 == 0 {
                let gap = rank[from] + (span - 1) / 2;
                gap_need[gap] = gap_need[gap].max(size_along(label) + 2.0 * LABEL_MARGIN);
            }
        }
    }
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); ranks.max(1)];
    // First order: depth first from the first shapes, in the order they came.
    let mut seen = vec![false; slots.len()];
    let mut seg_out: Vec<Vec<usize>> = vec![Vec::new(); slots.len()];
    let mut seg_in: Vec<Vec<usize>> = vec![Vec::new(); slots.len()];
    for (a, b) in &segments {
        seg_out[*a].push(*b);
        seg_in[*b].push(*a);
    }
    let mut visit_order = Vec::with_capacity(slots.len());
    for start in (0..n).filter(|node| rank[*node] == 0).chain(0..slots.len()) {
        let mut stack = vec![start];
        while let Some(slot) = stack.pop() {
            if seen[slot] {
                continue;
            }
            seen[slot] = true;
            visit_order.push(slot);
            for next in seg_out[slot].iter().rev() {
                stack.push(*next);
            }
        }
    }
    for slot in visit_order {
        layers[layer_of[slot]].push(slot);
    }

    // 4. Fewer crossings: sweep down and up, sorting each rank by where its
    // neighbours sit, and keep the best order seen.
    let crossings = |layers: &Vec<Vec<usize>>| -> usize {
        let mut position = vec![0usize; slots.len()];
        for layer in layers {
            for (index, slot) in layer.iter().enumerate() {
                position[*slot] = index;
            }
        }
        let mut total = 0;
        for l in 0..layers.len().saturating_sub(1) {
            let between: Vec<(usize, usize)> = segments.iter().filter(|(a, _)| layer_of[*a] == l).map(|(a, b)| (position[*a], position[*b])).collect();
            for i in 0..between.len() {
                for j in i + 1..between.len() {
                    let (a, b) = (between[i], between[j]);
                    if (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1) {
                        total += 1;
                    }
                }
            }
        }
        total
    };
    let mut best = layers.clone();
    let mut best_crossings = crossings(&layers);
    for iteration in 0..12 {
        let down = iteration % 2 == 0;
        let mut position = vec![0f64; slots.len()];
        for layer in &layers {
            for (index, slot) in layer.iter().enumerate() {
                position[*slot] = index as f64;
            }
        }
        let order: Vec<usize> = if down { (1..layers.len()).collect() } else { (0..layers.len().saturating_sub(1)).rev().collect() };
        for l in order {
            let mut keyed: Vec<(f64, usize)> = layers[l]
                .iter()
                .map(|slot| {
                    let neighbours = if down { &seg_in[*slot] } else { &seg_out[*slot] };
                    let key = if neighbours.is_empty() { position[*slot] } else { neighbours.iter().map(|other| position[*other]).sum::<f64>() / neighbours.len() as f64 };
                    (key, *slot)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[l] = keyed.into_iter().map(|(_, slot)| slot).collect();
            for (index, slot) in layers[l].iter().enumerate() {
                position[*slot] = index as f64;
            }
        }
        let count = crossings(&layers);
        if count < best_crossings {
            best_crossings = count;
            best = layers.clone();
        }
    }
    let layers = best;

    // 5. Across the ranks: each slot as close as it can get to the middle of
    // its neighbours, in order, never closer than the gap to the next.
    let gap_between = |a: usize, b: usize| if slots[a].node.is_some() && slots[b].node.is_some() { NODE_GAP } else { 14.0 };
    let mut center = vec![0f64; slots.len()];
    for layer in &layers {
        let mut at = 0.0;
        for (index, slot) in layer.iter().enumerate() {
            if index > 0 {
                at += slots[layer[index - 1]].across / 2.0 + gap_between(layer[index - 1], *slot) + slots[*slot].across / 2.0;
            }
            center[*slot] = at;
        }
        let middle = at / 2.0;
        for slot in layer {
            center[*slot] -= middle;
        }
    }
    for pass in 0..16 {
        let use_in = pass % 3 != 1;
        let use_out = pass % 3 != 0;
        let order: Vec<usize> = if pass % 2 == 0 { (0..layers.len()).collect() } else { (0..layers.len()).rev().collect() };
        for l in order {
            let layer = &layers[l];
            let desired: Vec<f64> = layer
                .iter()
                .map(|slot| {
                    let mut neighbours: Vec<usize> = Vec::new();
                    if use_in {
                        neighbours.extend(&seg_in[*slot]);
                    }
                    if use_out {
                        neighbours.extend(&seg_out[*slot]);
                    }
                    if neighbours.is_empty() {
                        center[*slot]
                    } else {
                        neighbours.iter().map(|other| center[*other]).sum::<f64>() / neighbours.len() as f64
                    }
                })
                .collect();
            let mut offsets = vec![0f64; layer.len()];
            for index in 1..layer.len() {
                offsets[index] = offsets[index - 1] + slots[layer[index - 1]].across / 2.0 + gap_between(layer[index - 1], layer[index]) + slots[layer[index]].across / 2.0;
            }
            let targets: Vec<f64> = desired.iter().zip(&offsets).map(|(d, o)| d - o).collect();
            let solved = isotonic(&targets);
            for (index, slot) in layer.iter().enumerate() {
                center[*slot] = solved[index] + offsets[index];
            }
        }
    }

    // 6. Along the ranks: each rank as thick as its thickest slot, the gaps
    // wide enough for the labels in them.
    let thickness: Vec<f64> = layers.iter().map(|layer| layer.iter().map(|slot| slots[*slot].along).fold(0.0, f64::max)).collect();
    let mut start = vec![0f64; layers.len()];
    for l in 1..layers.len() {
        start[l] = start[l - 1] + thickness[l - 1] + gap_need[l - 1];
    }
    let along_center = |slot: usize| start[layer_of[slot]] + thickness[layer_of[slot]] / 2.0;
    let point = |slot: usize| if direction == Direction::Right { (along_center(slot), center[slot]) } else { (center[slot], along_center(slot)) };
    let positions: Vec<(f64, f64)> = (0..n)
        .map(|node| {
            let (cx, cy) = point(node);
            (cx - graph.sizes[node].0 / 2.0, cy - graph.sizes[node].1 / 2.0)
        })
        .collect();
    let mut bends: Vec<Vec<(f64, f64)>> = chains.iter().enumerate().map(|(index, chain)| {
        let mut points: Vec<(f64, f64)> = chain.iter().map(|slot| point(*slot)).collect();
        if reversed[index] {
            points.reverse();
        }
        points
    }).collect();
    // Two arrows between the same shapes, one each way: bow the second so
    // they do not draw over each other.
    let pairs: HashSet<(usize, usize)> = graph.edges.iter().map(|(a, b, _)| (*a, *b)).collect();
    for (index, (from, to, _)) in graph.edges.iter().enumerate() {
        if from < to || !pairs.contains(&(*to, *from)) || !bends[index].is_empty() {
            continue;
        }
        let ((ax, ay), (bx, by)) = (center_of(&positions, graph, *from), center_of(&positions, graph, *to));
        let (mx, my) = ((ax + bx) / 2.0, (ay + by) / 2.0);
        let length = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt().max(1.0);
        bends[index] = vec![(mx - (by - ay) / length * 36.0, my + (bx - ax) / length * 36.0)];
    }
    finish(graph, direction, positions, bends)
}

fn center_of(positions: &[(f64, f64)], graph: &Graph, node: usize) -> (f64, f64) {
    (positions[node].0 + graph.sizes[node].0 / 2.0, positions[node].1 + graph.sizes[node].1 / 2.0)
}

/// The nondecreasing sequence closest to `targets` in least squares (pool
/// adjacent violators).
fn isotonic(targets: &[f64]) -> Vec<f64> {
    let mut blocks: Vec<(f64, usize)> = Vec::new();
    for target in targets {
        blocks.push((*target, 1));
        while blocks.len() > 1 && blocks[blocks.len() - 2].0 > blocks[blocks.len() - 1].0 {
            let (value, count) = blocks.pop().unwrap();
            let last = blocks.last_mut().unwrap();
            last.0 = (last.0 * last.1 as f64 + value * count as f64) / (last.1 + count) as f64;
            last.1 += count;
        }
    }
    blocks.into_iter().flat_map(|(value, count)| std::iter::repeat(value).take(count)).collect()
}

/// Ids grouped into the connected parts of a graph, by index.
pub fn components(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut Vec<usize>, mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for (a, b) in edges {
        let (ra, rb) = (root(&mut parent, *a), root(&mut parent, *b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for node in 0..n {
        let r = root(&mut parent, node);
        groups.entry(r).or_default().push(node);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort_by_key(|group| group[0]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes_overlap(layout: &Layout, graph: &Graph) -> bool {
        let n = graph.sizes.len();
        (0..n).any(|a| {
            (a + 1..n).any(|b| {
                let ((ax, ay), (aw, ah)) = (layout.positions[a], graph.sizes[a]);
                let ((bx, by), (bw, bh)) = (layout.positions[b], graph.sizes[b]);
                ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
            })
        })
    }

    #[test]
    fn text_is_measured_and_wrapped_as_excalidraw_does() {
        // Measured in the editor: "Login form" is 100.8 px at 20 px.
        assert!((text_width("Login form", 20.0) - 100.8).abs() < 0.01);
        assert_eq!(wrap("Deploy to production now", 150.0, 20.0), vec!["Deploy to", "production now"]);
        assert_eq!(wrap("one\ntwo", 500.0, 20.0), vec!["one", "two"]);
        // A word longer than a line breaks between characters.
        assert!(wrap("Supercalifragilistic", 80.0, 20.0).len() > 1);
        let (w, h) = fit_shape("rectangle", Some("Login form"));
        assert!(w >= 100.8 + 10.0 && h >= 25.0 + 10.0, "{w}x{h}");
        // A long label wraps rather than making a very wide box.
        let (w, h) = fit_shape("rectangle", Some("Waits for quota, re-checked every 30 seconds until room frees"));
        assert!(w <= 320.0 && h > 60.0, "{w}x{h}");
        // A diamond's label gets half its width.
        let (w, _) = fit_shape("diamond", Some("Valid?"));
        assert!(w / 2.0 - 10.0 >= text_width("Valid?", 20.0), "{w}");
    }

    #[test]
    fn a_branching_flow_is_layered_without_overlaps_or_crossings() {
        // login -> check -> (error | twofa) ; twofa -> (code | home) ; code -> home ; error -> login
        let sizes = vec![(140.0, 60.0), (170.0, 90.0), (120.0, 60.0), (150.0, 90.0), (120.0, 60.0), (130.0, 60.0)];
        let edges = vec![(0, 1, None), (1, 2, Some((40.0, 20.0))), (1, 3, Some((30.0, 20.0))), (3, 4, None), (3, 5, None), (4, 5, None), (2, 0, Some((60.0, 20.0)))];
        let graph = Graph { sizes, edges };
        for direction in [Direction::Right, Direction::Down] {
            let layout = layered(&graph, direction);
            assert!(!boxes_overlap(&layout, &graph), "{direction:?}: {layout:?}");
            // code sits between twofa and home: the twofa -> home arrow bends around it.
            assert!(!layout.bends[4].is_empty(), "{:?}", layout.bends);
        }
        let right = layered(&graph, Direction::Right);
        // Ranks run left to right along the arrows.
        assert!(right.positions[0].0 < right.positions[1].0 && right.positions[1].0 < right.positions[3].0 && right.positions[3].0 < right.positions[4].0);
    }

    #[test]
    fn gaps_leave_room_for_arrow_labels() {
        let graph = Graph { sizes: vec![(120.0, 60.0), (120.0, 60.0)], edges: vec![(0, 1, Some((150.0, 40.0)))] };
        let layout = layered(&graph, Direction::Right);
        let gap = layout.positions[1].0 - (layout.positions[0].0 + 120.0);
        assert!(gap >= 150.0 + 2.0 * LABEL_MARGIN, "{gap}");
    }

    #[test]
    fn a_tree_reads_down_and_a_long_chain_winds_to_fit_the_page() {
        let tree = Graph { sizes: vec![(120.0, 60.0); 7], edges: vec![(0, 1, None), (0, 2, None), (1, 3, None), (1, 4, None), (2, 5, None), (2, 6, None)] };
        let layout = arrange(&tree, None);
        assert_eq!(layout.direction, Direction::Down);
        assert!(layout.positions[0].1 < layout.positions[1].1 && layout.positions[1].1 < layout.positions[3].1);

        let chain = Graph { sizes: vec![(150.0, 60.0); 8], edges: (0..7).map(|i| (i, i + 1, None)).collect() };
        let straight = layered(&chain, Direction::Right);
        let best = arrange(&chain, None);
        assert!(best.scale() > straight.scale() * 1.3, "{} vs {}", best.scale(), straight.scale());
        assert!(!boxes_overlap(&best, &chain));
        assert!(page_scale(best.width, best.height) * LABEL_FONT >= 12.0, "{best:?}");
    }

    #[test]
    fn cycles_and_pairs_of_opposite_arrows_lay_out() {
        let graph = Graph { sizes: vec![(120.0, 60.0); 3], edges: vec![(0, 1, None), (1, 0, None), (1, 2, None), (2, 0, None)] };
        let layout = arrange(&graph, None);
        assert!(!boxes_overlap(&layout, &graph));
        // The return arrow bows away from the forward one.
        assert_eq!(layout.bends[1].len(), 1);
    }

    #[test]
    fn isotonic_keeps_order_and_stays_close() {
        assert_eq!(isotonic(&[1.0, 3.0, 2.0, 4.0]), vec![1.0, 2.5, 2.5, 4.0]);
        assert_eq!(components(4, &[(0, 1), (2, 3)]), vec![vec![0, 1], vec![2, 3]]);
    }
}
