// Scoring for the visualize eval. Pure: a case and what the harness did go
// in, a verdict comes out. The runner (../eval-visualize.mjs) only collects
// observations; every judgement lives here so it can be tested as data.

export const REFUSAL_PREFIX = "The visual was not drawn.";

/** The gates a prompt or tool-description change must pass. */
export const GATES = {
  precision: 0.95,
  recall: 0.9,
  formFit: 0.9,
  firstTryValid: 0.95,
  validAfterRetry: 0.995,
  untraced: 0,
  traps: 0.95,
  repeats: 0,
  overuse: 1.5,
};

const NUMBER = /(-?\$?\d[\d,]*(?:\.\d+)?)\s*(%|percent|k\b|K\b|thousand|m\b|M\b|million|mn\b|b\b|B\b|bn\b|billion)?/g;

/** Every value a source text supports, with the unit readings a writer could mean. */
export function sourceValues(text) {
  const values = [];
  for (const match of text.matchAll(NUMBER)) {
    const base = Number(match[1].replace(/[$,]/g, ""));
    if (!Number.isFinite(base)) continue;
    values.push(base);
    const unit = (match[2] ?? "").toLowerCase();
    if (unit === "%" || unit === "percent") values.push(base / 100);
    if (unit === "k" || unit === "thousand") values.push(base * 1e3);
    if (unit === "m" || unit === "million" || unit === "mn") values.push(base * 1e6);
    if (unit === "b" || unit === "bn" || unit === "billion") values.push(base * 1e9);
  }
  return values;
}

/** Whether a drawn value is one the source text states, within rounding. */
export function traces(value, supported) {
  return supported.some(candidate => {
    if (candidate === value) return true;
    const tolerance = Math.max(Math.abs(candidate) * 0.005, 0.5 * (Number.isInteger(value) && !Number.isInteger(candidate) ? 1 : 0));
    return Math.abs(candidate - value) <= tolerance;
  });
}

const blocksOf = spec => (Array.isArray(spec?.blocks) ? spec.blocks : []);
const sourceKind = (spec, id) => (Array.isArray(spec?.sources) ? spec.sources.find(source => source?.id === id)?.kind : undefined);

/**
 * Numeric values a spec presents as fact: chart rows, metric values, table
 * cells. Rows marked estimate, and blocks resting on computed or estimate
 * sources only, are reported separately rather than as facts.
 */
export function drawnValues(spec) {
  const facts = [];
  const estimates = [];
  for (const block of blocksOf(spec)) {
    const cited = Array.isArray(block.sourceIds) ? block.sourceIds : [];
    const derived = cited.length > 0 && cited.every(id => ["computed", "estimate"].includes(sourceKind(spec, id)));
    const push = (value, label, estimate) => {
      if (typeof value !== "number" || !Number.isFinite(value)) return;
      (estimate || derived ? estimates : facts).push({ value, label });
    };
    if (block.family === "chart") {
      for (const row of block.vegaLite?.data?.values ?? []) {
        const label = Object.values(row).filter(cell => typeof cell === "string").join(" ");
        for (const [key, cell] of Object.entries(row)) if (key !== "total") push(cell, `${label} ${key}`.trim(), row.estimate === true);
      }
    }
    if (block.family === "document" && block.form === "metric") {
      for (const item of block.content?.items ?? []) {
        const numeric = typeof item.value === "number" ? item.value : Number(String(item.value ?? "").replace(/[$,%\s]/g, ""));
        push(numeric, String(item.label ?? ""), item.estimate === true);
      }
    }
    if (block.family === "document" && block.form === "table") {
      for (const row of block.content?.rows ?? []) {
        const label = Object.values(row).filter(cell => typeof cell === "string").join(" ");
        for (const [key, cell] of Object.entries(row)) if (key !== "estimate") push(cell, `${label} ${key}`.trim(), row.estimate === true);
      }
    }
  }
  return { facts, estimates };
}

function formsOf(spec) {
  return blocksOf(spec).map(block => block?.form).filter(form => typeof form === "string");
}

/**
 * Score one case. `observation` is what the harness did on the case's last
 * turn: `calls` in order, each `{ input, refused }`.
 */
export function scoreCase(testCase, observation) {
  const calls = observation.calls ?? [];
  const called = calls.length > 0;
  const accepted = calls.find(call => !call.refused);
  const expectRender = testCase.expect.render === true;
  const result = {
    id: testCase.id,
    category: testCase.category,
    expectRender,
    called,
    decisionCorrect: called === expectRender,
    firstTryValid: called ? !calls[0].refused : null,
    validAfterRetry: called ? !!accepted : null,
    forms: accepted ? formsOf(accepted.input) : [],
    formFit: null,
    untraced: [],
    trapPassed: null,
  };
  if (accepted && expectRender && Array.isArray(testCase.expect.forms)) {
    result.formFit = result.forms.some(form => testCase.expect.forms.includes(form));
  }
  if (accepted && testCase.expect.traced) {
    const supported = sourceValues(testCase.turns.map(turn => turn.text).join("\n"));
    const { facts, estimates } = drawnValues(accepted.input);
    result.untraced = facts.filter(fact => !traces(fact.value, supported)).map(fact => `${fact.label}=${fact.value}`);
    const wanted = testCase.expect.estimates ?? [];
    let marked = true;
    if (wanted.includes("*")) {
      marked = facts.length === 0 && estimates.length > 0;
    } else {
      for (const label of wanted) {
        const stated = facts.some(fact => fact.label.toLowerCase().includes(label.toLowerCase()));
        if (stated) marked = false;
      }
    }
    result.trapPassed = result.untraced.length === 0 && marked;
  }
  return result;
}

const ratio = (part, whole) => (whole === 0 ? null : part / whole);

/** The table a run reports, with each gate's verdict. */
export function aggregate(results, cases) {
  const positives = results.filter(result => result.called);
  const truePositives = positives.filter(result => result.expectRender);
  const expected = results.filter(result => result.expectRender);
  const fits = results.filter(result => result.formFit !== null);
  const called = results.filter(result => result.called);
  const traced = results.filter(result => result.trapPassed !== null);
  const repeats = results.filter(result => result.category === "multi-turn" && !result.expectRender && result.called);
  // Overuse: how much more often a form is chosen than the dataset expects.
  const byId = new Map(cases.map(testCase => [testCase.id, testCase]));
  const chosen = new Map();
  const wanted = new Map();
  for (const result of results) {
    const testCase = byId.get(result.id);
    if (result.forms[0]) chosen.set(result.forms[0], (chosen.get(result.forms[0]) ?? 0) + 1);
    const primary = testCase?.expect?.render ? testCase.expect.forms?.[0] : undefined;
    if (primary) wanted.set(primary, (wanted.get(primary) ?? 0) + 1);
  }
  const chosenTotal = [...chosen.values()].reduce((sum, count) => sum + count, 0);
  const wantedTotal = [...wanted.values()].reduce((sum, count) => sum + count, 0);
  let overuse = 0;
  let overused = null;
  for (const [form, count] of chosen) {
    const share = count / Math.max(1, chosenTotal);
    const expectedShare = Math.max((wanted.get(form) ?? 0) / Math.max(1, wantedTotal), 1 / Math.max(1, wantedTotal));
    if (share / expectedShare > overuse) {
      overuse = share / expectedShare;
      overused = form;
    }
  }
  const metrics = {
    cases: results.length,
    precision: ratio(truePositives.length, positives.length),
    recall: ratio(truePositives.length, expected.length),
    formFit: ratio(fits.filter(result => result.formFit).length, fits.length),
    firstTryValid: ratio(called.filter(result => result.firstTryValid).length, called.length),
    validAfterRetry: ratio(called.filter(result => result.validAfterRetry).length, called.length),
    untraced: results.reduce((sum, result) => sum + result.untraced.length, 0),
    traps: ratio(traced.filter(result => result.trapPassed).length, traced.length),
    repeats: repeats.length,
    overuse: chosenTotal ? Number(overuse.toFixed(2)) : null,
    overused,
  };
  const pass = {
    precision: metrics.precision === null || metrics.precision >= GATES.precision,
    recall: metrics.recall === null || metrics.recall >= GATES.recall,
    formFit: metrics.formFit === null || metrics.formFit >= GATES.formFit,
    firstTryValid: metrics.firstTryValid === null || metrics.firstTryValid >= GATES.firstTryValid,
    validAfterRetry: metrics.validAfterRetry === null || metrics.validAfterRetry >= GATES.validAfterRetry,
    untraced: metrics.untraced <= GATES.untraced,
    traps: metrics.traps === null || metrics.traps >= GATES.traps,
    repeats: metrics.repeats <= GATES.repeats,
    overuse: metrics.overuse === null || metrics.overuse <= GATES.overuse,
  };
  return { metrics, pass };
}
