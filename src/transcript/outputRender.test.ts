import { describe, expect, it } from "vitest";
import { detectStructuredOutput, ghSummary, listRows, repairTruncatedJson, runSummary } from "./outputRender";

const PR = {
  number: 70,
  title: "test(104): reproduce session over-length crash",
  state: "OPEN",
  author: { login: "Atharva-Kanherkar" },
  headRefName: "fix/session-overlength",
  baseRefName: "main",
  url: "https://github.com/Atharva-Kanherkar/kairo/pull/70",
  body: "## Validation\n\n| Check | Result |\n|---|---|\n| Harness | PASS |\n",
  files: [
    { path: "tools/repro.py", additions: 188, deletions: 0, changeType: "ADDED" },
    { path: "README.md", additions: 9, deletions: 4, changeType: "MODIFIED" },
  ],
  commits: [{ oid: "a" }, { oid: "b" }],
  reviews: [],
};

describe("detectStructuredOutput", () => {
  it("recognises a PR from gh pr view, wrapped or not", () => {
    const output = JSON.stringify(PR);
    expect(detectStructuredOutput("gh pr view 70 --json title,body,files", output)?.kind).toBe("gh-pr");
    expect(detectStructuredOutput(`/bin/zsh -lc "gh pr view 70 --json title"`, output)?.kind).toBe("gh-pr");
  });

  it("recognises an issue, a run and a list", () => {
    expect(detectStructuredOutput("gh issue view 12 --json title,body", JSON.stringify({ number: 12, title: "x" }))?.kind).toBe("gh-issue");
    expect(detectStructuredOutput("gh run view 9 --json jobs", JSON.stringify({ status: "completed", jobs: [] }))?.kind).toBe("gh-run");
    expect(detectStructuredOutput("gh pr list --json number,title,state", JSON.stringify([{ number: 1, title: "a", state: "OPEN" }]))?.kind).toBe("gh-list");
  });

  it("falls back to a JSON tree for anything else that parses", () => {
    expect(detectStructuredOutput("cat package.json", '{"name":"x"}')?.kind).toBe("json");
    // A PR-shaped document from a command that is not gh is still just JSON.
    expect(detectStructuredOutput("cat pr.json", JSON.stringify(PR))?.kind).toBe("json");
  });

  it("leaves text alone", () => {
    expect(detectStructuredOutput("ls", "a\nb\n")).toBeNull();
    expect(detectStructuredOutput("echo", "42")).toBeNull();
    expect(detectStructuredOutput("gh pr view 70", undefined)).toBeNull();
  });

  it("repairs a PR cut off by head -c and says so", () => {
    const full = JSON.stringify(PR);
    const cut = full.slice(0, full.indexOf("Harness") + 4);
    const detected = detectStructuredOutput("gh pr view 70 --json title,body | head -c 20000", cut);
    expect(detected?.kind).toBe("gh-pr");
    expect(detected?.truncated).toBe(true);
    expect(ghSummary(detected?.data).title).toBe(PR.title);
    expect(ghSummary(detected?.data).body).toContain("| Check | Result |\n");
  });

  it("gives up on malformed rather than truncated JSON", () => {
    expect(detectStructuredOutput("cat x", '{"a": [1, 2}')).toBeNull();
  });

  it("does not parse oversize output", () => {
    expect(detectStructuredOutput("cat big.json", `[${"1,".repeat(300_000)}1]`)).toBeNull();
  });
});

describe("repairTruncatedJson", () => {
  it.each([
    ['{"a": "hel', { a: "hel" }],
    ['{"a": 1, "b":', { a: 1 }],
    ['{"a": 1, "b', { a: 1 }],
    ['{"a": [1, 2, tr', { a: [1, 2] }],
    ['{"a": [{"x": 1}, {"y": -1.', { a: [{ x: 1 }, {}] }],
    ['[{"a": "x\\', [{ a: "x" }]],
    ['{"a": {"b": "c"},', { a: { b: "c" } }],
  ])("closes %s", (text, expected) => {
    expect(JSON.parse(repairTruncatedJson(text)!)).toEqual(expected);
  });

  it("refuses a complete document", () => {
    expect(repairTruncatedJson('{"a": 1}')).toBeUndefined();
  });
});

describe("summaries", () => {
  it("reads a PR into card fields", () => {
    expect(ghSummary(PR)).toMatchObject({
      repo: "Atharva-Kanherkar/kairo", number: 70, state: "OPEN", author: "Atharva-Kanherkar",
      head: "fix/session-overlength", base: "main", additions: 197, deletions: 4, changedFiles: 2, commits: 2, reviews: 0,
    });
    expect(ghSummary(PR).files[0]).toEqual({ path: "tools/repro.py", additions: 188, deletions: 0, change: "A" });
  });

  it("tallies a status check rollup", () => {
    const checks = ghSummary({ statusCheckRollup: [{ conclusion: "SUCCESS" }, { conclusion: "FAILURE" }, { status: "IN_PROGRESS" }] }).checks;
    expect(checks).toEqual({ passed: 1, failed: 1, pending: 1 });
  });

  it("reads labels from either shape", () => {
    expect(ghSummary({ labels: [{ name: "bug" }] }).labels).toEqual(["bug"]);
    expect(ghSummary({ labels: { nodes: [{ name: "ux" }] } }).labels).toEqual(["ux"]);
  });

  it("reads a run and a list", () => {
    expect(runSummary({ displayTitle: "CI", status: "completed", conclusion: "failure", jobs: [{ name: "rust", conclusion: "failure" }] }).jobs)
      .toEqual([{ name: "rust", conclusion: "failure", status: undefined }]);
    expect(listRows([{ number: 3, title: "a", state: "OPEN" }, { databaseId: 9, displayTitle: "CI", conclusion: "success" }]))
      .toEqual([{ id: "#3", title: "a", state: "OPEN" }, { id: "9", title: "CI", state: "success" }]);
  });
});
