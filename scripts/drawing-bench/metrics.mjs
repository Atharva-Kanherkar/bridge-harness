// Scores one completed Excalidraw scene: does it hold what was asked for, and
// can a person read it as the page shows it. Pure geometry over the elements
// Excalidraw itself completed (`convertToExcalidrawElements` in a browser), so
// label sizes and container growth are the editor's own, not an estimate.
//
// Defects a person sees at a glance are "severe": a label outside its box,
// two boxes on top of each other, text over a box or over other text, an
// arrow cutting through a box it does not join, an arrow too short to show
// its head, and text too small to read once the drawing fits the page.
// Crossing arrows and arrows bound to nothing are "minor".

/** The note column a drawing preview fills, and the preview's height cap
 *  (`DrawingPreview`: `w-full max-h-[28rem]` inside a 44rem article with px-8). */
export const PREVIEW = { width: 640, height: 448, padding: 16 };
export const SEVERE = ["label_overflow", "overlap", "text_overlap", "arrow_through", "short_arrow", "tiny_text"];
const SHAPES = new Set(["rectangle", "ellipse", "diamond", "image", "embeddable", "iframe"]);
const LINEAR = new Set(["arrow", "line"]);
/** Below this many pixels on screen, text is not readable. */
const MIN_READABLE_PX = 9;

const live = elements => elements.filter(e => e && !e.isDeleted);

export function box(e) {
  const x = Math.min(e.x, e.x + (e.width ?? 0));
  const y = Math.min(e.y, e.y + (e.height ?? 0));
  return { x, y, w: Math.abs(e.width ?? 0), h: Math.abs(e.height ?? 0) };
}

const inflate = (b, d) => ({ x: b.x - d, y: b.y - d, w: Math.max(0, b.w + 2 * d), h: Math.max(0, b.h + 2 * d) });
const intersection = (a, b) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
const inside = (inner, outer, slack = 1) => inner.x >= outer.x - slack && inner.y >= outer.y - slack && inner.x + inner.w <= outer.x + outer.w + slack && inner.y + inner.h <= outer.y + outer.h + slack;
const containsPoint = (b, [px, py], slack = 0) => px >= b.x - slack && px <= b.x + b.w + slack && py >= b.y - slack && py <= b.y + b.h + slack;
const center = b => [b.x + b.w / 2, b.y + b.h / 2];

/** A linear element's points in scene coordinates. */
export function points(e) {
  return (e.points ?? []).map(([px, py]) => [e.x + px, e.y + py]);
}

function segmentsCross([a, b], [c, d]) {
  const cross = (o, p, q) => (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0]);
  const d1 = cross(c, d, a), d2 = cross(c, d, b), d3 = cross(a, b, c), d4 = cross(a, b, d);
  return ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0));
}

function segmentHitsBox([a, b], r) {
  if (containsPoint(r, a) || containsPoint(r, b)) return true;
  const corners = [[r.x, r.y], [r.x + r.w, r.y], [r.x + r.w, r.y + r.h], [r.x, r.y + r.h]];
  return corners.some((corner, i) => segmentsCross([a, b], [corner, corners[(i + 1) % 4]]));
}

const segments = pts => pts.slice(1).map((p, i) => [pts[i], p]);
const length = pts => segments(pts).reduce((sum, [a, b]) => sum + Math.hypot(b[0] - a[0], b[1] - a[1]), 0);
const norm = text => String(text ?? "").toLowerCase().replace(/\s+/g, " ").trim();

/** The scene, indexed: shapes, texts by role, links and their ends. */
export function index(elements) {
  const all = live(elements);
  const byId = new Map(all.map(e => [e.id, e]));
  const shapes = all.filter(e => SHAPES.has(e.type));
  const texts = all.filter(e => e.type === "text");
  const links = all.filter(e => LINEAR.has(e.type));
  const labelOf = new Map();
  for (const t of texts) if (t.containerId && byId.has(t.containerId)) labelOf.set(t.containerId, t);
  const label = e => {
    if (e.type === "text") return norm(e.originalText ?? e.text);
    const bound = labelOf.get(e.id);
    return norm(bound?.originalText ?? bound?.text ?? e.label?.text ?? e.name ?? "");
  };
  const shapeAt = (point, skip) => shapes
    .filter(s => s.id !== skip && containsPoint(box(s), point, 12))
    .sort((a, b) => box(a).w * box(a).h - box(b).w * box(b).h)[0];
  const ends = link => {
    const pts = points(link);
    const end = (binding, point) => {
      const id = link[binding]?.elementId ?? link[binding === "startBinding" ? "start" : "end"]?.id;
      if (id && byId.has(id)) return { id, bound: true };
      const hit = point && shapeAt(point, link.id);
      return { id: hit?.id ?? null, bound: false };
    };
    return [end("startBinding", pts[0]), end("endBinding", pts[pts.length - 1])];
  };
  return { all, byId, shapes, texts, links, labelOf, label, ends };
}

/** Every defect in the scene, each with the ids it involves. */
export function defects(elements) {
  const scene = index(elements);
  const found = [];
  const add = (kind, ids, detail) => found.push({ kind, ids: [...ids].sort(), detail });
  const { shapes, texts, links, byId } = scene;

  for (const t of texts) {
    const container = t.containerId && byId.get(t.containerId);
    if (container && SHAPES.has(container.type) && !inside(box(t), box(container), 2)) {
      add("label_overflow", [container.id], `"${scene.label(container).slice(0, 40)}" spills out of its ${container.type}`);
    }
  }

  for (let i = 0; i < shapes.length; i++) {
    for (let j = i + 1; j < shapes.length; j++) {
      const [a, b] = [box(shapes[i]), box(shapes[j])];
      const shared = intersection(inflate(a, -2), inflate(b, -2));
      // One box wholly inside another is a group, not a collision.
      if (shared > 0 && !inside(a, b) && !inside(b, a)) add("overlap", [shapes[i].id, shapes[j].id], `"${scene.label(shapes[i]).slice(0, 30)}" and "${scene.label(shapes[j]).slice(0, 30)}" overlap`);
    }
  }

  // Text over a box it does not belong to, or over other text.
  const owner = t => (t.containerId && byId.get(t.containerId)) || null;
  for (const t of texts) {
    const tb = inflate(box(t), -1);
    const own = owner(t);
    const onArrow = own && LINEAR.has(own.type);
    for (const s of shapes) {
      if (own && own.id === s.id) continue;
      const sb = box(s);
      if (intersection(tb, inflate(sb, -2)) <= 0) continue;
      // Free text written inside a box (a legend, a note in a frame) is fine;
      // a label of another box or of an arrow sitting on this box is not.
      if (!own && inside(box(t), sb)) continue;
      if (own && !onArrow && inside(box(own), sb)) continue;
      add("text_overlap", [t.containerId ?? t.id, s.id], `${onArrow ? "arrow label" : own ? "label" : "text"} "${norm(t.originalText ?? t.text).slice(0, 30)}" sits on "${scene.label(s).slice(0, 30)}"`);
    }
  }
  for (let i = 0; i < texts.length; i++) {
    for (let j = i + 1; j < texts.length; j++) {
      if (intersection(inflate(box(texts[i]), -1), inflate(box(texts[j]), -1)) > 0) {
        add("text_overlap", [texts[i].containerId ?? texts[i].id, texts[j].containerId ?? texts[j].id], `"${norm(texts[i].text).slice(0, 24)}" and "${norm(texts[j].text).slice(0, 24)}" overlap`);
      }
    }
  }

  for (const link of links) {
    const pts = points(link);
    const [start, end] = scene.ends(link);
    if (link.type === "arrow" && length(pts) < 24) add("short_arrow", [link.id], `arrow ${link.id} is ${Math.round(length(pts))}px long`);
    if (!start.bound || !end.bound) add("dangling", [link.id], `${link.type} ${link.id} is not attached at ${!start.bound && !end.bound ? "either end" : !start.bound ? "its start" : "its end"}`);
    for (const s of shapes) {
      if (s.id === start.id || s.id === end.id) continue;
      const sb = box(s);
      // An arrow drawn inside a grouping box does not cut through it.
      if (containsPoint(sb, pts[0]) || containsPoint(sb, pts[pts.length - 1])) continue;
      if (segments(pts).some(seg => segmentHitsBox(seg, inflate(sb, -4)))) add("arrow_through", [link.id, s.id], `${link.type} ${link.id} cuts through "${scene.label(s).slice(0, 30)}"`);
    }
  }
  for (let i = 0; i < links.length; i++) {
    for (let j = i + 1; j < links.length; j++) {
      const [a, b] = [segments(points(links[i])), segments(points(links[j]))];
      if (a.some(sa => b.some(sb => segmentsCross(sa, sb)))) add("crossing", [links[i].id, links[j].id], `${links[i].id} crosses ${links[j].id}`);
    }
  }

  const fit = readability(elements);
  if (fit.minPx !== null && fit.minPx < MIN_READABLE_PX) add("tiny_text", [], `smallest text is ${fit.minPx.toFixed(1)}px once the drawing fits the page (scale ${fit.scale.toFixed(2)})`);
  return found;
}

/** How large the drawing's text is once the preview fits it to the page. */
export function readability(elements) {
  const all = live(elements);
  if (!all.length) return { width: 0, height: 0, scale: 1, minPx: null, medianPx: null };
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const e of all) {
    const pts = LINEAR.has(e.type) || e.type === "freedraw" ? points(e) : [];
    const b = box(e);
    for (const [px, py] of [[b.x, b.y], [b.x + b.w, b.y + b.h], ...pts]) {
      x0 = Math.min(x0, px); y0 = Math.min(y0, py); x1 = Math.max(x1, px); y1 = Math.max(y1, py);
    }
  }
  const width = x1 - x0 + 2 * PREVIEW.padding, height = y1 - y0 + 2 * PREVIEW.padding;
  const scale = Math.min(PREVIEW.width / width, PREVIEW.height / height);
  const sizes = all.filter(e => e.type === "text").map(e => (e.fontSize ?? 20) * scale).sort((a, b) => a - b);
  return { width: Math.round(width), height: Math.round(height), scale, minPx: sizes[0] ?? null, medianPx: sizes.length ? sizes[Math.floor(sizes.length / 2)] : null };
}

/** Does the drawing hold what the request asked for? Each check scores 0 or 1. */
export function coverage(elements, expect = {}) {
  const scene = index(elements);
  const candidates = [...scene.shapes, ...scene.texts.filter(t => !t.containerId)];
  const matches = words => candidates.filter(e => words.some(word => scene.label(e).includes(norm(word))));
  const nodeMatches = (expect.nodes ?? []).map(matches);
  const checks = [];
  nodeMatches.forEach((found, i) => checks.push({ kind: "node", ok: found.length > 0, detail: (expect.nodes[i] ?? []).join("|") }));

  const pairs = scene.links.map(link => scene.ends(link).map(end => end.id));
  for (const [a, b] of expect.edges ?? []) {
    const from = a < 0 ? scene.shapes.map(s => s.id) : (nodeMatches[a] ?? []).map(e => e.id);
    const to = b < 0 ? scene.shapes.map(s => s.id) : (nodeMatches[b] ?? []).map(e => e.id);
    const ok = pairs.some(([s, e]) => s && e && s !== e && ((from.includes(s) && to.includes(e)) || (from.includes(e) && to.includes(s))));
    checks.push({ kind: "edge", ok, detail: `${a < 0 ? "*" : expect.nodes[a][0]} - ${b < 0 ? "*" : expect.nodes[b][0]}` });
  }

  const allText = scene.all.filter(e => e.type === "text").map(e => norm(e.originalText ?? e.text)).join(" \n ");
  for (const words of expect.labels ?? []) checks.push({ kind: "label", ok: words.some(word => allText.includes(norm(word))), detail: words.join("|") });

  if (expect.stack) {
    const ys = expect.stack.map(group => group.map(i => nodeMatches[i]?.[0]).filter(Boolean).map(e => center(box(e))[1]));
    const ok = ys.every(g => g.length) && ys.slice(1).every((g, i) => Math.min(...g) > Math.max(...ys[i]));
    checks.push({ kind: "stack", ok, detail: "groups run top to bottom" });
  }
  if (expect.distinct) {
    const colors = expect.distinct.map(i => nodeMatches[i]?.[0]).map(e => e ? `${e.backgroundColor}|${e.strokeColor}` : null);
    checks.push({ kind: "distinct", ok: colors.every(Boolean) && new Set(colors).size === colors.length, detail: "status colors differ" });
  }
  for (const id of expect.keep ?? []) checks.push({ kind: "keep", ok: scene.byId.has(id), detail: id });
  for (const text of expect.drop ?? []) checks.push({ kind: "drop", ok: !candidates.some(e => scene.label(e).includes(norm(text))), detail: text });

  const passed = checks.filter(c => c.ok).length;
  return { checks, passed, total: checks.length, score: checks.length ? passed / checks.length : 1 };
}

const key = d => `${d.kind}:${d.ids.join(",")}`;

/** The full report for one drawing. With `seed` (the person's drawing before
 *  an edit), defects already in the seed are not charged to the agent. */
export function score(elements, expect, seed) {
  const all = defects(elements);
  const inherited = new Set(seed ? defects(seed).map(key) : []);
  const own = all.filter(d => !inherited.has(key(d)));
  const counts = Object.fromEntries([...SEVERE, "crossing", "dangling"].map(kind => [kind, own.filter(d => d.kind === kind).length]));
  const severe = own.filter(d => SEVERE.includes(d.kind)).length;
  const cov = coverage(elements, expect);
  const fit = readability(elements);
  return {
    coverage: cov,
    defects: own,
    counts,
    severe,
    clean: severe === 0,
    pass: severe === 0 && cov.score >= 0.9,
    readability: fit,
    elements: live(elements).filter(e => !(e.type === "text" && e.containerId)).length,
  };
}
