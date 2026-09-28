import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  EXEMPT_LABEL,
  EXEMPT_MARKER,
  MIN_SECTION_CHARS,
  checkIssueFormat,
  parseHeadings,
  rejectionComment,
} from "../issue-format.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));

const HUMANS = "A short summary of the shape of the problem plus a diagram.";
const AGENTS = "Touch src-tauri/bridge-core/src/agent.rs and add a regression test.";

function body({ humans = HUMANS, agents = AGENTS, order = "humans-first" } = {}) {
  const humanBlock = `## For humans\n\n${humans}\n`;
  const agentBlock = `## For agents\n\n${agents}\n`;
  return order === "humans-first"
    ? `${humanBlock}\n${agentBlock}`
    : `${agentBlock}\n${humanBlock}`;
}

test("accepts a well-formed two-audience body", () => {
  const result = checkIssueFormat(body());
  assert.equal(result.ok, true);
  assert.equal(result.exempt, false);
  assert.deepEqual(result.problems, []);
  assert.deepEqual(result.sections, { humans: true, agents: true });
});

test("rejects a body with no sections at all", () => {
  const result = checkIssueFormat("opencode is broken, streaming stalls, please fix");
  assert.equal(result.ok, false);
  assert.equal(result.problems.length, 2);
  assert.match(result.problems[0], /For humans/);
  assert.match(result.problems[1], /For agents/);
});

test("rejects a body that only addresses one audience", () => {
  const onlyHumans = `## For humans\n\n${HUMANS}\n`;
  const result = checkIssueFormat(onlyHumans);
  assert.equal(result.ok, false);
  assert.deepEqual(result.sections, { humans: true, agents: false });
});

test("rejects sections that exist but are empty", () => {
  const result = checkIssueFormat("## For humans\n\n## For agents\n\n");
  assert.equal(result.ok, false);
  assert.equal(result.problems.length, 2);
  for (const problem of result.problems) assert.match(problem, /empty or too thin/);
});

test("does not count template guidance as written content", () => {
  const filled = `## For humans\n\n<!-- TL;DR, a diagram, the architecture. -->\n\n- [ ]\n\nTODO\n\n## For agents\n\n${AGENTS}\n`;
  const result = checkIssueFormat(filled);
  assert.equal(result.ok, false);
  assert.equal(result.sections.humans, false);
  assert.equal(result.sections.agents, true);
});

test("requires humans before agents", () => {
  const result = checkIssueFormat(body({ order: "agents-first" }));
  assert.equal(result.ok, false);
  assert.deepEqual(result.problems, ["`For humans` must come before `For agents`."]);
});

test("tolerates emoji, h3, singular wording, and trailing colons", () => {
  const decorated = `### 🧑‍🦰 For Human:\n\n${HUMANS}\n\n### 🤖 For Agent\n\n${AGENTS}\n`;
  assert.equal(checkIssueFormat(decorated).ok, true);
});

test("ignores headings deeper than h3 so subsections cannot satisfy the rule", () => {
  const nested = `## Context\n\n#### For humans\n\n${HUMANS}\n\n#### For agents\n\n${AGENTS}\n`;
  const result = checkIssueFormat(nested);
  assert.equal(result.ok, false);
  assert.deepEqual(result.sections, { humans: false, agents: false });
});

test("a section ends where the next heading begins", () => {
  const headings = parseHeadings(body());
  assert.deepEqual(
    headings.map((heading) => heading.title),
    ["For humans", "For agents"],
  );
  assert.equal(headings[0].body.includes(AGENTS), false);
  assert.equal(headings[1].body.includes(AGENTS), true);
});

test("the label escape hatch exempts an otherwise invalid body", () => {
  const result = checkIssueFormat("nothing here", { labels: ["bug", EXEMPT_LABEL] });
  assert.equal(result.ok, true);
  assert.equal(result.exempt, true);
});

test("the label escape hatch is case-insensitive", () => {
  assert.equal(checkIssueFormat("nothing", { labels: ["Format-Exempt"] }).exempt, true);
});

test("the inline marker exempts automation-authored bodies", () => {
  const result = checkIssueFormat(`<!-- ${EXEMPT_MARKER} -->\nautomated report`);
  assert.equal(result.ok, true);
  assert.equal(result.exempt, true);
});

test("a null or empty body fails rather than throwing", () => {
  for (const value of [null, undefined, ""]) {
    const result = checkIssueFormat(value);
    assert.equal(result.ok, false);
    assert.equal(result.problems.length, 2);
  }
});

test("content just under and just over the threshold flips the verdict", () => {
  const thin = "x".repeat(MIN_SECTION_CHARS - 1);
  const thick = "x".repeat(MIN_SECTION_CHARS);
  assert.equal(checkIssueFormat(body({ humans: thin })).ok, false);
  assert.equal(checkIssueFormat(body({ humans: thick })).ok, true);
});

test("the rejection comment names every problem and the escape hatch", () => {
  const { problems } = checkIssueFormat("");
  const comment = rejectionComment(problems);
  for (const problem of problems) assert.ok(comment.includes(problem));
  assert.ok(comment.includes(EXEMPT_LABEL));
  assert.ok(comment.includes("## For humans"));
  assert.ok(comment.includes("## For agents"));
});

// A template cannot pass the content threshold — it has no content yet. What it
// must do is put the two headings in the right order, so that a contributor who
// fills it in top to bottom passes the gate without thinking about it.
test("every shipped issue template carries both headings in order", () => {
  const templates = join(root, ".github/ISSUE_TEMPLATE");
  const files = readdirSync(templates).filter((name) => name.endsWith(".md"));
  assert.ok(files.length > 0, "expected markdown issue templates");
  for (const file of files) {
    const raw = readFileSync(join(templates, file), "utf8").replace(/^---\n[\s\S]*?\n---\n/, "");
    const audience = parseHeadings(raw)
      .filter((heading) => /for (humans?|agents?)/i.test(heading.title))
      .map((heading) => heading.title.toLowerCase().includes("human"));
    assert.deepEqual(audience, [true, false], `${file}: expected For humans then For agents`);
  }
});
