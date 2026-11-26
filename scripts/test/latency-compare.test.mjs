import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(new URL("../latency-compare.mjs", import.meta.url));

function report({ newChat = 100, forest = 2, fixtureVersion = 1 } = {}) {
  return {
    fixtureVersion,
    metrics: {
      new_chat: { samples: 60, p50Ms: 80, p95Ms: newChat },
      warm_send: { samples: 60, p50Ms: 3, p95Ms: 5 },
      forest_entry: { samples: 60, p50Ms: 0.5, p95Ms: forest },
      app_state: { samples: 60, p50Ms: 20, p95Ms: 30 },
    },
  };
}

function compare(base, head) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-latency-compare-"));
  try {
    const paths = [base, head].map((rounds, index) => {
      const path = join(dir, `${index}.jsonl`);
      writeFileSync(path, rounds.map((round) => JSON.stringify(round)).join("\n"));
      return path;
    });
    return spawnSync(process.execPath, [script, ...paths], { encoding: "utf8" });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const three = (overrides) => Array.from({ length: 3 }, () => report(overrides));

test("a single noisy round and exact-margin changes pass", () => {
  const result = compare(three(), [
    report({ newChat: 120, forest: 3 }),
    report({ newChat: 120, forest: 3 }),
    report({ newChat: 900, forest: 90 }),
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /new_chat.*100\.00.*120\.00.*300/);
});

test("a sustained small slowdown above the allowed margin fails", () => {
  const result = compare(three(), three({ newChat: 120.1 }));
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stdout, /new_chat.*regression/);
});

test("a persistence regression fails even if another metric improves", () => {
  const result = compare(three(), three({ newChat: 90, forest: 52 }));
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stdout, /forest_entry.*regression/);
});

test("an unchanged over-budget baseline appears in the report", () => {
  const result = compare(three({ newChat: 310 }), three({ newChat: 310 }));
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /new_chat.*over target/);
});

test("incomplete or incomparable input fails closed", () => {
  const missing = three();
  delete missing[0].metrics.forest_entry;
  const tooFew = three();
  tooFew[0].metrics.warm_send.samples = 59;
  const invalid = three();
  invalid[0].metrics.app_state.p95Ms = null;
  for (const [head, diagnostic] of [
    [three().slice(0, 2), /head.*3 rounds/],
    [three({ fixtureVersion: 2 }), /fixture version/],
    [missing, /head.*forest_entry/],
    [tooFew, /head.*warm_send.*samples/],
    [invalid, /head.*app_state.*p95Ms/],
  ]) {
    const result = compare(three(), head);
    assert.equal(result.status, 1);
    assert.match(result.stderr, diagnostic);
  }
});
