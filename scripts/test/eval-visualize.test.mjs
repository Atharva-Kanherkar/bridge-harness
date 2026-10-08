// The visualize eval's deterministic half: the dataset is well formed and the
// scorer judges correctly. The model-driven half runs with
// `bun run eval:visualize`, because it spends tokens.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { aggregate, drawnValues, scoreCase, sourceValues, traces } from "../eval-visualize/score.mjs";

const root = join(import.meta.dirname, "../..");
const cases = readFileSync(join(root, "testing/evals/visualize/cases.jsonl"), "utf8").trim().split("\n").map(line => JSON.parse(line));
const forms = JSON.parse(readFileSync(join(root, "testing/fixtures/visualize/forms.json"), "utf8"));

test("cases parse and keep the agreed mix", () => {
  const ids = new Set();
  for (const testCase of cases) {
    assert.ok(!ids.has(testCase.id), `duplicate ${testCase.id}`);
    ids.add(testCase.id);
    assert.ok(["render", "no-render", "multi-turn", "grounding"].includes(testCase.category), testCase.id);
    assert.equal(testCase.turns.at(-1).role, "user", testCase.id);
    assert.equal(typeof testCase.expect.render, "boolean", testCase.id);
  }
  const count = category => cases.filter(testCase => testCase.category === category).length;
  assert.ok(cases.length >= 250, `${cases.length} cases`);
  assert.ok(count("render") >= 90);
  assert.ok(count("no-render") >= 90);
  assert.ok(count("multi-turn") >= 40);
  assert.ok(count("grounding") >= 30);
});

test("cases reference known forms and each render case accepts one this build draws", () => {
  const known = new Set(forms.map(info => info.form));
  const available = new Set(forms.filter(info => info.available).map(info => info.form));
  for (const testCase of cases.filter(testCase => testCase.expect.render)) {
    assert.ok(Array.isArray(testCase.expect.forms) && testCase.expect.forms.length > 0, testCase.id);
    for (const form of testCase.expect.forms) assert.ok(known.has(form), `${testCase.id}: ${form}`);
    assert.ok(testCase.expect.forms.some(form => available.has(form)), `${testCase.id} needs an available form`);
  }
});

test("no expected form carries more than 15% of the render category's labels", () => {
  // The should-render set is the one built for variety. Grounding traps and
  // multi-turn follow-ups lean on bar and line by nature and are not counted.
  const primaries = cases.filter(testCase => testCase.category === "render").map(testCase => testCase.expect.forms[0]);
  const counts = new Map();
  for (const form of primaries) counts.set(form, (counts.get(form) ?? 0) + 1);
  for (const [form, count] of counts) assert.ok(count / primaries.length <= 0.15, `${form}: ${count}/${primaries.length}`);
});

test("source values cover the unit readings a writer could mean", () => {
  const values = sourceValues("Tokyo 37.2 million, growth 14%, revenue $4.3 billion, 12.4k stars, 1,860 signups");
  for (const value of [37.2, 37_200_000, 14, 0.14, 4.3, 4_300_000_000, 12.4, 12_400, 1860]) {
    assert.ok(traces(value, values), String(value));
  }
  assert.ok(traces(88, sourceValues("Alpha 88.1")), "rounding to a whole number traces");
  assert.ok(!traces(91, sourceValues("Alpha 88.1")), "an invented value does not");
});

const spec = (rows, sources = [{ id: "s", kind: "web", ref: "https://a.example" }]) => ({
  version: 1, title: "t", sources,
  blocks: [{ family: "chart", form: "line", sourceIds: sources.map(source => source.id), vegaLite: { mark: "line", data: { values: rows }, encoding: { x: { field: "year" }, y: { field: "v" } } } }],
});

test("drawn values separate facts from estimates", () => {
  const { facts, estimates } = drawnValues(spec([{ year: "2023", v: 2.9 }, { year: "2024", v: 3.4, estimate: true }]));
  assert.deepEqual(facts.map(fact => fact.value), [2.9]);
  assert.deepEqual(estimates.map(fact => fact.value), [3.4]);
});

const ground = cases.find(testCase => testCase.id === "ground-001");

test("a grounding trap passes when missing years are omitted or marked", () => {
  const omitted = scoreCase(ground, { calls: [{ refused: false, input: spec([{ year: "2020", v: 1.1 }, { year: "2021", v: 1.6 }, { year: "2022", v: 2.2 }, { year: "2023", v: 2.9 }]) }] });
  assert.equal(omitted.trapPassed, true);
  const marked = scoreCase(ground, { calls: [{ refused: false, input: spec([{ year: "2023", v: 2.9 }, { year: "2024", v: 3.5, estimate: true }]) }] });
  assert.equal(marked.trapPassed, true);
  const invented = scoreCase(ground, { calls: [{ refused: false, input: spec([{ year: "2023", v: 2.9 }, { year: "2024", v: 3.5 }]) }] });
  assert.equal(invented.trapPassed, false);
  assert.deepEqual(invented.untraced, ["2024 v=3.5"]);
});

test("decision, validity and form fit are scored per case", () => {
  const render = cases.find(testCase => testCase.category === "render");
  const quiet = cases.find(testCase => testCase.category === "no-render");
  const drawn = { refused: false, input: { blocks: [{ family: "chart", form: render.expect.forms[0] }] } };
  const hit = scoreCase(render, { calls: [{ refused: true, input: {} }, drawn] });
  assert.equal(hit.decisionCorrect, true);
  assert.equal(hit.firstTryValid, false);
  assert.equal(hit.validAfterRetry, true);
  assert.equal(hit.formFit, true);
  assert.equal(scoreCase(quiet, { calls: [] }).decisionCorrect, true);
  assert.equal(scoreCase(quiet, { calls: [drawn] }).decisionCorrect, false);
});

test("the aggregate applies every gate", () => {
  const render = cases.find(testCase => testCase.category === "render");
  const quiet = cases.find(testCase => testCase.category === "no-render");
  const drawn = { refused: false, input: { blocks: [{ family: "chart", form: render.expect.forms[0] }] } };
  const perfect = aggregate([scoreCase(render, { calls: [drawn] }), scoreCase(quiet, { calls: [] })], [render, quiet]);
  assert.equal(perfect.metrics.precision, 1);
  assert.equal(perfect.metrics.recall, 1);
  assert.ok(Object.values(perfect.pass).every(Boolean), JSON.stringify(perfect));
  const noisy = aggregate([scoreCase(render, { calls: [drawn] }), scoreCase(quiet, { calls: [drawn] })], [render, quiet]);
  assert.equal(noisy.metrics.precision, 0.5);
  assert.equal(noisy.pass.precision, false);
});
