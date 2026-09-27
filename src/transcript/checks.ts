/**
 * Which shell commands are checks — a test run, a build, a typecheck, a lint —
 * and what their output says happened.
 *
 * A check is the one command whose result a reader actually wants from a run:
 * "216 tests passed" is the news, the other forty commands are the work. The
 * transcript draws these the way the Verifying card draws its checks, so the
 * two surfaces speak one language. React-free, so it tests as data.
 */

export type CheckKind = "test" | "build" | "typecheck" | "lint";

export interface CheckFacet {
  kind: CheckKind;
  /** What the output reports, when it is in a shape we recognise. */
  summary?: string;
  /** The output reported failures, whatever the exit code said. */
  failures?: boolean;
}

export const CHECK_LABEL: Record<CheckKind, string> = {
  test: "Test",
  build: "Build",
  typecheck: "Typecheck",
  lint: "Lint",
};

/** Runners that only launch the real program: `bunx vitest` is vitest. */
const RUNNERS: string[][] = [["bunx"], ["npx"], ["pnpx"], ["pnpm", "exec"], ["pnpm", "dlx"], ["yarn", "dlx"], ["uv", "run"], ["poetry", "run"], ["bun", "x"]];
const PACKAGE_MANAGERS = new Set(["bun", "npm", "pnpm", "yarn"]);

function tokens(segment: string): string[] {
  return segment.trim().split(/\s+/).filter(Boolean);
}

/** The program and its arguments once env assignments and runners are gone. */
function program(segment: string): string[] {
  let words = tokens(segment);
  while (words.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0])) words = words.slice(1);
  if (words[0] === "sudo" || words[0] === "command" || words[0] === "time") words = words.slice(1);
  for (const runner of RUNNERS) {
    if (runner.every((word, index) => words[index] === word)) { words = words.slice(runner.length); break; }
  }
  if (words.length) words[0] = words[0].split("/").pop() ?? words[0];
  return words;
}

/** A package script (`bun run test:unit`, `npm test`), as its script name. */
function script(words: string[]): string | undefined {
  if (!PACKAGE_MANAGERS.has(words[0])) return undefined;
  const rest = words.slice(1).filter(word => !word.startsWith("-"));
  if (rest[0] === "run" || rest[0] === "run-script") return rest[1];
  // `bun test` is bun's own runner, handled with the programs below.
  if (words[0] === "bun" && rest[0] === "test") return undefined;
  return rest[0];
}

function scriptKind(name: string): CheckKind | undefined {
  const base = name.split(":")[0];
  if (/^(test|tests|e2e|spec)$/.test(base)) return "test";
  if (/^build$/.test(base)) return "build";
  if (/^(check|typecheck|type-check|tsc|types)$/.test(base)) return "typecheck";
  if (/^(lint|oxlint|eslint)$/.test(base)) return "lint";
  return undefined;
}

function programKind(words: string[]): CheckKind | undefined {
  const [bin, sub, ...rest] = words;
  const args = [sub, ...rest].filter(Boolean);
  switch (bin) {
    case "cargo":
      if (sub === "test" || sub === "nextest") return "test";
      if (sub === "build") return "build";
      if (sub === "check") return "typecheck";
      if (sub === "clippy") return "lint";
      return undefined;
    case "go":
      return sub === "test" ? "test" : sub === "build" ? "build" : sub === "vet" ? "lint" : undefined;
    case "swift":
      return sub === "test" ? "test" : sub === "build" ? "build" : undefined;
    case "bun":
    case "deno":
      return sub === "test" ? "test" : undefined;
    case "node":
      return args.includes("--test") ? "test" : undefined;
    case "python":
    case "python3":
      return sub === "-m" && rest[0] === "pytest" ? "test" : sub === "-m" && rest[0] === "mypy" ? "typecheck" : undefined;
    case "vitest":
    case "jest":
    case "pytest":
    case "rspec":
    case "playwright":
      return bin === "playwright" ? (sub === "test" ? "test" : undefined) : "test";
    case "vite":
      return sub === "build" ? "build" : undefined;
    case "tsc":
      if (args.includes("--noEmit")) return "typecheck";
      return args.includes("-b") || args.includes("--build") || !args.length ? "build" : "typecheck";
    case "make":
      return sub && /^(test|check)/.test(sub) ? "test" : "build";
    case "xcodebuild":
      return args.includes("test") ? "test" : "build";
    case "mypy":
    case "pyright":
      return "typecheck";
    case "eslint":
    case "oxlint":
    case "biome":
    case "ruff":
    case "golangci-lint":
      return "lint";
    default:
      return undefined;
  }
}

/// The check a command runs, read off the first segment that is not a `cd`.
/// A chain like `cd app && bun run build && bun run test` is named by its first
/// check, which is also what fails first.
export function classifyCheck(command: string): CheckKind | undefined {
  const segments = command.split(/\s*(?:&&|;|\|\|)\s*/).map(segment => segment.split(/\s\|\s/)[0]);
  for (const segment of segments) {
    const words = program(segment);
    if (!words.length || words[0] === "cd" || words[0] === "pushd") continue;
    const name = script(words);
    return name !== undefined ? scriptKind(name) : programKind(words);
  }
  return undefined;
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

function counts(passed: number, failed: number, noun = "test", failedFiles = 0): { summary: string; failures: boolean } {
  if (failed > 0) return { summary: `${failed} failed · ${passed} passed`, failures: true };
  // A file that fails to collect runs none of its tests, so the test tally
  // alone can read all-green. The file count is what says the run failed.
  if (failedFiles > 0) return { summary: `${plural(failedFiles, "file")} failed · ${passed} passed`, failures: true };
  return { summary: `${plural(passed, noun)} passed`, failures: false };
}

/// What a check's output says, in one short phrase. Only shapes we recognise:
/// an unrecognised log gets no summary rather than a guessed one.
export function summarizeCheckOutput(kind: CheckKind, output: string | undefined): Pick<CheckFacet, "summary" | "failures"> {
  if (!output) return {};
  const text = output.replace(/\u001b\[[0-9;]*m/g, "");
  if (kind === "test") {
    // cargo: one `test result:` line per test binary; sum them.
    const cargo = [...text.matchAll(/test result: \w+\. (\d+) passed; (\d+) failed/g)];
    if (cargo.length) return counts(cargo.reduce((n, m) => n + Number(m[1]), 0), cargo.reduce((n, m) => n + Number(m[2]), 0));
    // vitest: `Tests  1 failed | 470 passed (471)`.
    const vitestFailed = /^\s*Tests\s+(\d+) failed/m.exec(text);
    const vitestPassed = /^\s*Tests\s+(?:\d+ failed \| )?(\d+) passed/m.exec(text);
    const vitestFiles = /^\s*Test Files\s+(\d+) failed/m.exec(text);
    if (vitestFailed || vitestPassed || vitestFiles) return counts(Number(vitestPassed?.[1] ?? 0), Number(vitestFailed?.[1] ?? 0), "test", Number(vitestFiles?.[1] ?? 0));
    // jest: `Tests:       1 failed, 41 passed, 42 total`.
    const jest = /Tests:\s+(?:(\d+) failed, )?(?:\d+ skipped, )?(\d+) passed/.exec(text);
    const jestSuites = /Test Suites:\s+(\d+) failed/.exec(text);
    if (jest || jestSuites) return counts(Number(jest?.[2] ?? 0), Number(jest?.[1] ?? 0), "test", Number(jestSuites?.[1] ?? 0));
    // pytest: `=== 3 failed, 41 passed in 2.1s ===`.
    const pytest = /=+ (?:(\d+) failed, )?(\d+) passed/.exec(text);
    if (pytest) return counts(Number(pytest[2]), Number(pytest[1] ?? 0));
    // bun test: ` 41 pass` / ` 1 fail` on their own lines.
    const bunPass = /^\s*(\d+) pass\s*$/m.exec(text);
    if (bunPass) return counts(Number(bunPass[1]), Number(/^\s*(\d+) fail\s*$/m.exec(text)?.[1] ?? 0));
    // node --test: `# pass 12` / `# fail 0`.
    const nodePass = /^# pass (\d+)/m.exec(text);
    if (nodePass) return counts(Number(nodePass[1]), Number(/^# fail (\d+)/m.exec(text)?.[1] ?? 0));
    return {};
  }
  const tsc = /Found (\d+) errors?/.exec(text);
  if (tsc) return { summary: plural(Number(tsc[1]), "error"), failures: Number(tsc[1]) > 0 };
  // rustc's own tally lines ("aborting due to…", "could not compile") are not errors.
  const rustErrors = (text.match(/^error(?:\[E\d+\])?: (?!aborting|could not compile)/gm) ?? []).length;
  if (rustErrors) return { summary: plural(rustErrors, "error"), failures: true };
  const lint = /(\d+) problems? \((\d+) errors?/.exec(text) ?? /Found (\d+) warnings? and (\d+) errors?/.exec(text);
  if (lint) return { summary: lint[0].startsWith("Found") ? `${plural(Number(lint[2]), "error")} · ${plural(Number(lint[1]), "warning")}` : `${plural(Number(lint[2]), "error")}`, failures: Number(lint[2]) > 0 };
  const vite = /built in ([\d.]+m?s)/.exec(text);
  if (vite) return { summary: `built in ${vite[1]}`, failures: false };
  const cargo = /Finished .* in ([\d.]+m?s)/.exec(text);
  if (cargo) return { summary: `finished in ${cargo[1]}`, failures: false };
  return {};
}

export function readCheck(command: string | undefined, output: string | undefined): CheckFacet | undefined {
  if (!command) return undefined;
  const kind = classifyCheck(command);
  return kind ? { kind, ...summarizeCheckOutput(kind, output) } : undefined;
}
