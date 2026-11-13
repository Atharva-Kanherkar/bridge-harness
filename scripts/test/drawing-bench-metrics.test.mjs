import assert from "node:assert/strict";
import test from "node:test";
import { coverage, defects, readability, score } from "../drawing-bench/metrics.mjs";

// Completed elements, shaped as Excalidraw leaves them after it completes a scene.
const rect = (id, x, y, w = 160, h = 60, extra = {}) => ({ id, type: "rectangle", x, y, width: w, height: h, backgroundColor: "transparent", strokeColor: "#1e1e1e", ...extra });
const label = (container, text, x, y, w, h = 25) => ({ id: `${container}~label`, type: "text", x, y, width: w, height: h, text, originalText: text, fontSize: 20, containerId: container });
const arrow = (id, from, to, pts, extra = {}) => ({ id, type: "arrow", x: pts[0][0], y: pts[0][1], width: 0, height: 0, points: pts.map(([x, y]) => [x - pts[0][0], y - pts[0][1]]), startBinding: from && { elementId: from }, endBinding: to && { elementId: to }, ...extra });

test("a clean two-box flow has no defects and full coverage", () => {
  const elements = [rect("a", 0, 0), label("a", "Login", 40, 17, 80), rect("b", 300, 0), label("b", "Home", 40 + 300, 17, 80), arrow("e", "a", "b", [[166, 30], [294, 30]])];
  assert.deepEqual(defects(elements), []);
  const cov = coverage(elements, { nodes: [["login"], ["home"]], edges: [[0, 1]] });
  assert.equal(cov.score, 1);
  assert.equal(score(elements, { nodes: [["login"], ["home"]], edges: [[0, 1]] }).pass, true);
});

test("overlapping boxes, a label outside its box and an arrow label on a box are severe", () => {
  const elements = [
    rect("a", 0, 0), label("a", "A very long label that spills", -20, 17, 220),
    rect("b", 120, 20),
    rect("c", 500, 0), arrow("e", "b", "c", [[280, 50], [494, 30]]),
    { id: "e~label", type: "text", x: 230, y: 10, width: 200, height: 50, text: "a long arrow label", originalText: "a long arrow label", fontSize: 20, containerId: "e" },
  ];
  const kinds = defects(elements).map(d => d.kind);
  assert.ok(kinds.includes("overlap"), kinds.join());
  assert.ok(kinds.includes("label_overflow"), kinds.join());
  assert.ok(kinds.includes("text_overlap"), kinds.join());
});

test("an arrow through a box it does not join is caught; one inside a group box is not", () => {
  const through = [rect("a", 0, 0), rect("mid", 250, 0), rect("b", 500, 0), arrow("e", "a", "b", [[166, 30], [494, 30]])];
  assert.deepEqual(defects(through).filter(d => d.kind === "arrow_through").map(d => d.ids), [["e", "mid"]]);
  const grouped = [rect("group", -40, -40, 800, 200), rect("a", 0, 0), rect("b", 500, 0), arrow("e", "a", "b", [[166, 30], [494, 30]])];
  assert.deepEqual(defects(grouped).filter(d => d.kind === "arrow_through"), []);
});

test("a drawing too wide for the page reads as tiny text", () => {
  const wide = Array.from({ length: 10 }, (_, i) => rect(`n${i}`, i * 300, 0)).concat([{ id: "t", type: "text", x: 0, y: 100, width: 50, height: 25, text: "hi", fontSize: 20 }]);
  const fit = readability(wide);
  assert.ok(fit.minPx < 9, String(fit.minPx));
  assert.ok(defects(wide).some(d => d.kind === "tiny_text"));
});

test("coverage scores nodes, edges, stacking, colors, kept and dropped elements", () => {
  const elements = [
    rect("top", 0, 0, 160, 60, { backgroundColor: "#b2f2bb" }), label("top", "CEO Maya", 20, 17, 120),
    rect("low", 0, 200, 160, 60, { backgroundColor: "#ffec99" }), label("low", "Raj", 60, 217, 40),
    arrow("e", "top", "low", [[80, 66], [80, 194]]),
  ];
  const cov = coverage(elements, { nodes: [["maya"], ["raj"], ["lena"]], edges: [[0, 1], [0, 2]], stack: [[0], [1]], distinct: [0, 1], keep: ["top", "gone"], drop: ["legacy"] });
  const byKind = Object.fromEntries(cov.checks.map(c => [`${c.kind}:${c.detail}`, c.ok]));
  assert.equal(byKind["node:lena"], false);
  assert.equal(byKind["edge:maya - raj"], true);
  assert.equal(byKind["edge:maya - lena"], false);
  assert.equal(byKind["stack:groups run top to bottom"], true);
  assert.equal(byKind["distinct:status colors differ"], true);
  assert.equal(byKind["keep:gone"], false);
  assert.equal(byKind["drop:legacy"], true);
});

test("defects already in the person's drawing are not charged to the agent", () => {
  const seed = [rect("a", 0, 0), rect("b", 100, 20)];
  const after = [...seed, rect("c", 600, 0)];
  assert.equal(score(after, {}, seed).severe, 0);
  assert.equal(score(after, {}).severe, 1);
});

test("shapes are judged by their outlines, and words broken mid-word are caught", () => {
  const diamond = { id: "d", type: "diamond", x: 0, y: 0, width: 200, height: 100 };
  // A label in the diamond's empty corner is not on it.
  const corner = { id: "t", type: "text", x: 2, y: 2, width: 30, height: 20, text: "Yes", originalText: "Yes", fontSize: 16 };
  assert.deepEqual(defects([diamond, corner]).filter(d => d.kind === "text_overlap"), []);
  const link = arrow("a", null, null, [[300, 300], [400, 300]]);
  const onArrow = { ...corner, containerId: "a" };
  assert.deepEqual(defects([diamond, link, onArrow]).filter(d => d.kind === "text_overlap"), []);
  const middle = { ...onArrow, x: 85, y: 40 };
  assert.equal(defects([diamond, link, middle]).filter(d => d.kind === "text_overlap").length, 1);
  const broken = [rect("e", 0, 0, 120, 80), { id: "e~label", type: "text", x: 5, y: 5, width: 110, height: 50, text: "Elasticsear\nch", originalText: "Elasticsearch", fontSize: 20, containerId: "e" }];
  assert.deepEqual(defects(broken).map(d => d.kind), ["broken_word"]);
  const wrapped = [rect("e", 0, 0, 120, 80), { id: "e~label", type: "text", x: 5, y: 5, width: 110, height: 50, text: "Search\nservice", originalText: "Search service", fontSize: 20, containerId: "e" }];
  assert.deepEqual(defects(wrapped), []);
  const diamondLabel = [rect("q", 0, 0, 200, 120), { id: "q~label", type: "text", x: 50, y: 20, width: 100, height: 75, text: "Need\ntransactio\nns?", originalText: "Need transactions?", fontSize: 20, containerId: "q" }];
  assert.deepEqual(defects(diamondLabel).map(d => d.kind), ["broken_word"]);
});
