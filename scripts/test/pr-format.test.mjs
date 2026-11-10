import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  EXEMPT_LABEL,
  EXEMPT_MARKER,
  MIN_SECTION_CHARS,
  SECTIONS,
  checkPullRequestFormat,
  failureMessage,
} from "../pr-format.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));

const OUTPUT = "```\n$ node --test scripts/test\n# pass 42\n# fail 0\n```";

const FILLED = {
  did: "Added a PR notes gate next to the title check.",
  codebase: "scripts/pr-format.mjs and .github/workflows/pr-title.yml.",
  users: "No user-facing change; contributors see one more check.",
  tests: `Ran the release tests.\n\n${OUTPUT}`,
  risks: "Bodies written before this rule fail until edited.",
};

function body(overrides = {}) {
  const parts = { ...FILLED, ...overrides };
  return SECTIONS.filter((section) => parts[section.key] !== null)
    .map((section) => `## ${section.heading}\n\n${parts[section.key]}\n`)
    .join("\n");
}

test("accepts a body with all five sections filled in", () => {
  const result = checkPullRequestFormat(body());
  assert.equal(result.ok, true, result.problems.join("\n"));
  assert.equal(result.exempt, false);
  assert.ok(Object.values(result.sections).every(Boolean));
});

test("names every missing section", () => {
  const result = checkPullRequestFormat("Fixes the stall. Tested locally.");
  assert.equal(result.ok, false);
  assert.equal(result.problems.length, SECTIONS.length);
  for (const section of SECTIONS) {
    assert.ok(result.problems.some((problem) => problem.includes(section.heading)), section.heading);
  }
});

test("rejects one missing section and keeps the others", () => {
  const result = checkPullRequestFormat(body({ users: null }));
  assert.equal(result.ok, false);
  assert.deepEqual(result.problems, ["Missing a `## Users affected` section."]);
  assert.equal(result.sections.users, false);
  assert.equal(result.sections.did, true);
});

test("the unfilled template fails on every section", () => {
  const template = readFileSync(`${root}.github/PULL_REQUEST_TEMPLATE.md`, "utf8");
  const result = checkPullRequestFormat(template);
  assert.equal(result.ok, false);
  assert.equal(result.problems.length, SECTIONS.length);
  assert.ok(result.problems.every((problem) => /empty or too thin/.test(problem)));
});

test("the template carries every heading the check looks for", () => {
  const template = readFileSync(`${root}.github/PULL_REQUEST_TEMPLATE.md`, "utf8");
  for (const section of SECTIONS) {
    assert.match(template, new RegExp(`^## ${section.heading}$`, "m"));
  }
});

test("tests need a fenced block of raw output, not a summary", () => {
  const result = checkPullRequestFormat(body({ tests: "All tests pass locally, vitest and cargo." }));
  assert.equal(result.ok, false);
  assert.equal(result.problems.length, 1);
  assert.match(result.problems[0], /unedited output in a fenced code block/);
});

test("an empty fence does not count as output", () => {
  const result = checkPullRequestFormat(body({ tests: "Ran everything locally.\n\n```\n\n```" }));
  assert.equal(result.ok, false);
  assert.match(result.problems[0], /fenced code block/);
});

test("headings inside pasted output do not split the section", () => {
  const noisy = "Ran it.\n\n```\n## Users affected\n# fail 0\n```";
  const result = checkPullRequestFormat(body({ tests: noisy }));
  assert.equal(result.ok, true, result.problems.join("\n"));
});

test("subsections stay inside their parent section", () => {
  const nested = `Ran the suites.\n\n### vitest\n\n${OUTPUT}\n\n### cargo\n\nskipped, no Rust change`;
  const result = checkPullRequestFormat(body({ tests: nested }));
  assert.equal(result.ok, true, result.problems.join("\n"));
});

test("tolerates emoji, h3, case and trailing colons", () => {
  const decorated = body()
    .replace("## What I did", "### 🛠 What I Did:")
    .replace("## Tradeoffs and risks", "## Trade-offs / what could go wrong");
  const result = checkPullRequestFormat(decorated);
  assert.equal(result.ok, true, result.problems.join("\n"));
});

test("content just under and just over the threshold flips the verdict", () => {
  const under = checkPullRequestFormat(body({ users: "x".repeat(MIN_SECTION_CHARS - 1) }));
  const over = checkPullRequestFormat(body({ users: "x".repeat(MIN_SECTION_CHARS) }));
  assert.equal(under.ok, false);
  assert.equal(over.ok, true);
});

test("bots, the label and the marker are exempt", () => {
  assert.equal(checkPullRequestFormat("bump deps", { isBot: true }).exempt, true);
  assert.equal(checkPullRequestFormat("", { labels: [EXEMPT_LABEL.toUpperCase()] }).exempt, true);
  assert.equal(checkPullRequestFormat(`<!-- ${EXEMPT_MARKER} -->`).exempt, true);
});

test("a null body fails rather than throwing", () => {
  assert.equal(checkPullRequestFormat(null).ok, false);
});

test("the failure message lists each problem and the escape hatch", () => {
  const message = failureMessage(["Missing a `## Tests I ran` section."]);
  assert.match(message, /- Missing a `## Tests I ran` section\./);
  assert.match(message, new RegExp(EXEMPT_LABEL));
  assert.match(message, /docs\/pr-format\.md/);
});
