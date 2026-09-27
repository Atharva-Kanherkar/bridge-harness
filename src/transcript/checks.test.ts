import { describe, expect, it } from "vitest";
import { classifyCheck, readCheck, summarizeCheckOutput } from "./checks";
import { readToolCall } from "./toolCall";

describe("classifyCheck", () => {
  it.each([
    ["cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core", "test"],
    ["bun test src/auth", "test"],
    ["bun run test", "test"],
    ["npm test", "test"],
    ["pnpm run test:unit", "test"],
    ["bunx vitest run src/App.test.tsx", "test"],
    ["npx jest", "test"],
    ["python -m pytest -q", "test"],
    ["go test ./...", "test"],
    ["node --test sidecar/test/a.mjs", "test"],
    ["cd sidecar && npm test", "test"],
    ["CI=1 bun run test", "test"],
    ["bun run build", "build"],
    ["cargo build --release", "build"],
    ["vite build", "build"],
    ["tsc -b", "build"],
    ["tsc --noEmit", "typecheck"],
    ["cargo check --workspace", "typecheck"],
    ["bun run check", "typecheck"],
    ["cargo clippy -- -D warnings", "lint"],
    ["bunx oxlint src", "lint"],
    ["bun run lint", "lint"],
  ])("reads `%s` as %s", (command, kind) => {
    expect(classifyCheck(command)).toBe(kind);
  });

  it.each(["ls src", "git status", "bun run dev", "cat package.json", "rg test src", "echo test"])("leaves `%s` alone", command => {
    expect(classifyCheck(command)).toBeUndefined();
  });
});

describe("summarizeCheckOutput", () => {
  it("sums cargo's per-binary results", () => {
    const output = "test result: ok. 200 passed; 0 failed; 0 ignored\n...\ntest result: ok. 16 passed; 0 failed; 1 ignored";
    expect(summarizeCheckOutput("test", output)).toEqual({ summary: "216 tests passed", failures: false });
  });

  it("reads vitest, including a run with failures", () => {
    expect(summarizeCheckOutput("test", " Test Files  212 passed (212)\n      Tests  2677 passed (2677)")).toEqual({ summary: "2677 tests passed", failures: false });
    expect(summarizeCheckOutput("test", "      Tests  1 failed | 470 passed (471)")).toEqual({ summary: "1 failed · 470 passed", failures: true });
    expect(summarizeCheckOutput("test", "      Tests  2 failed (2)")).toEqual({ summary: "2 failed · 0 passed", failures: true });
  });

  it("reads bun, jest, pytest and node --test", () => {
    expect(summarizeCheckOutput("test", " 41 pass\n 1 fail")).toEqual({ summary: "1 failed · 41 passed", failures: true });
    expect(summarizeCheckOutput("test", "Tests:       1 failed, 41 passed, 42 total")).toEqual({ summary: "1 failed · 41 passed", failures: true });
    expect(summarizeCheckOutput("test", "===== 3 failed, 41 passed in 2.10s =====")).toEqual({ summary: "3 failed · 41 passed", failures: true });
    expect(summarizeCheckOutput("test", "# tests 12\n# pass 12\n# fail 0")).toEqual({ summary: "12 tests passed", failures: false });
  });

  it("reads builds and typechecks", () => {
    expect(summarizeCheckOutput("build", "vite v5\n✓ built in 6.76s")).toEqual({ summary: "built in 6.76s", failures: false });
    expect(summarizeCheckOutput("typecheck", "Found 3 errors in 2 files.")).toEqual({ summary: "3 errors", failures: true });
    expect(summarizeCheckOutput("build", "error[E0308]: mismatched types\nerror: aborting due to 1 previous error")).toEqual({ summary: "1 error", failures: true });
  });

  it("says nothing about output it does not recognise", () => {
    expect(summarizeCheckOutput("test", "some log line")).toEqual({});
    expect(summarizeCheckOutput("build", undefined)).toEqual({});
  });
});

describe("check facet on tool calls", () => {
  it("stamps a run command with its check and summary", () => {
    const tool = readToolCall({ text: "", status: "completed", surface: "activity", data: { type: "commandExecution", command: "bun run test", exitCode: 0, aggregatedOutput: "Tests  2677 passed (2677)" } });
    expect(tool.check).toEqual({ kind: "test", summary: "2677 tests passed", failures: false });
    expect(readCheck("ls", "")).toBeUndefined();
  });
});
