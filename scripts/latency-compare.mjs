import { readFileSync } from "node:fs";

const ROUNDS = 3;
const SAMPLES = 60;
const RELATIVE_MARGIN = 0.2;
const metrics = {
  new_chat: { target: 300, floor: 5 },
  warm_send: { target: 50, floor: 5 },
  forest_entry: { target: 5, floor: 1 },
  app_state: { target: 150, floor: 5 },
};

function readRounds(path, side) {
  const input = readFileSync(path, "utf8").trim();
  const lines = input ? input.split(/\r?\n/) : [];
  if (lines.length !== ROUNDS) {
    throw new Error(`${side} needs ${ROUNDS} rounds, found ${lines.length}`);
  }
  return lines.map((line, index) => {
    const label = `${side} round ${index + 1}`;
    let round;
    try {
      round = JSON.parse(line);
    } catch {
      throw new Error(`${label} is invalid JSON`);
    }
    if (!Number.isSafeInteger(round?.fixtureVersion) || round.fixtureVersion < 1) {
      throw new Error(`${label} has an invalid fixture version`);
    }
    for (const name of Object.keys(metrics)) {
      const metric = round.metrics?.[name];
      if (metric?.samples !== SAMPLES) {
        throw new Error(`${label} ${name} must have ${SAMPLES} samples`);
      }
      if (!Number.isFinite(metric.p50Ms) || metric.p50Ms < 0) {
        throw new Error(`${label} ${name} has an invalid p50Ms`);
      }
      if (!Number.isFinite(metric.p95Ms) || metric.p95Ms < metric.p50Ms) {
        throw new Error(`${label} ${name} has an invalid p95Ms`);
      }
    }
    return round;
  });
}

function roundMedian(rounds, name) {
  return rounds.map(({ metrics }) => metrics[name].p95Ms).sort((a, b) => a - b)[1];
}

function main([basePath, headPath]) {
  if (!basePath || !headPath) {
    throw new Error("Usage: node scripts/latency-compare.mjs BASE.jsonl HEAD.jsonl");
  }
  const base = readRounds(basePath, "base");
  const head = readRounds(headPath, "head");
  const fixture = base[0].fixtureVersion;
  if ([...base, ...head].some((round) => round.fixtureVersion !== fixture)) {
    throw new Error("base and head rounds must use the same fixture version");
  }

  console.log(`### Send latency (fixture ${fixture}, median of ${ROUNDS} round p95s)`);
  console.log("");
  console.log("| Metric | Base (ms) | Head (ms) | Target (ms) | Margin (ms) | Result |");
  console.log("| --- | ---: | ---: | ---: | ---: | --- |");
  let failed = false;
  for (const [name, { target, floor }] of Object.entries(metrics)) {
    const before = roundMedian(base, name);
    const after = roundMedian(head, name);
    const margin = Math.max(before * RELATIVE_MARGIN, floor);
    const regression = after > before + margin;
    const result = [regression ? "regression" : "pass"];
    if (after > target) result.push("over target");
    console.log(`| ${name} | ${before.toFixed(2)} | ${after.toFixed(2)} | ${target} | ${margin.toFixed(2)} | ${result.join(", ")} |`);
    failed ||= regression;
  }
  return failed ? 1 : 0;
}

try {
  process.exitCode = main(process.argv.slice(2));
} catch (error) {
  console.error(`Latency comparison failed: ${error.message}`);
  process.exitCode = 1;
}
