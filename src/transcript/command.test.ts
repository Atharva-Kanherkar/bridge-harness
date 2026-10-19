import { describe, expect, it } from "vitest";
import { commandPhrase, leadStep, readCommand, splitShell, substitutions, unwrapShell } from "./command";

const steps = (command: string) => readCommand(command).steps;
const only = (command: string) => {
  const read = steps(command);
  expect(read).toHaveLength(1);
  return read[0];
};
const label = (command: string) => {
  const phrase = commandPhrase(readCommand(command))!;
  return `${phrase.done} ${phrase.target ?? ""}`.trim();
};

describe("unwrapShell", () => {
  it("unwraps Codex's zsh -lc with double-quoted escapes", () => {
    expect(unwrapShell(`/bin/zsh -lc "sed -n '1,20p' a.go; rg -n \\"x\\\\(\\" src"`)).toBe(`sed -n '1,20p' a.go; rg -n "x\\(" src`);
  });

  it("unwraps a single-quoted script and its spliced quotes", () => {
    expect(unwrapShell(`bash -lc 'echo '"'"'hi'"'"' && ls'`)).toBe(`echo 'hi' && ls`);
    expect(unwrapShell(`sh -c 'echo '\\''hi'\\'''`)).toBe(`echo 'hi'`);
  });

  it("leaves anything that is not exactly one script as typed", () => {
    expect(unwrapShell("bash script.sh")).toBe("bash script.sh");
    expect(unwrapShell(`zsh -lc "echo $1" arg`)).toBe(`zsh -lc "echo $1" arg`);
    expect(unwrapShell("ls -la")).toBe("ls -la");
  });
});

describe("splitShell", () => {
  it("never splits inside quotes, substitutions or subshells", () => {
    expect(splitShell(`grep "a;b" f && echo $(date; whoami) | (cd x; ls)`).map(piece => piece.text))
      .toEqual([`grep "a;b" f`, "echo $(date; whoami)", "(cd x; ls)"]);
  });

  it("keeps a heredoc body with the command that opened it", () => {
    const pieces = splitShell("cat > notes.md <<'EOF'\n# a; b && c\nEOF\nls");
    expect(pieces.map(piece => piece.text)).toEqual(["cat > notes.md <<'EOF'", "ls"]);
  });

  it("does not split on the & of a redirect", () => {
    expect(splitShell("cargo test 2>&1 &> log").map(piece => piece.text)).toEqual(["cargo test 2>&1 &> log"]);
  });
});

describe("notes", () => {
  it("turns the screenshot's plumbing into words", () => {
    const read = steps("ls tools/ 2>/dev/null; ls -la tools/vllm 2>/dev/null | head; ls tools/vllm-hunt 2>/dev/null | head -50; git -C tools/vllm log -1 --format='%H %cd' 2>/dev/null; cat .gitignore | head -50; df -h /Users/atharva | tail -1");
    expect(read.map(step => `${step.done} ${step.target}`)).toEqual([
      "Listed tools", "Listed vllm", "Listed vllm-hunt", "Viewed git log", "Read .gitignore", "Checked disk space",
    ]);
    expect(read.map(step => step.notes)).toEqual([
      ["errors hidden"],
      ["errors hidden", "first 10 lines"],
      ["errors hidden", "first 50 lines"],
      ["in tools/vllm", "errors hidden"],
      ["first 50 lines"],
      ["last 1 line"],
    ]);
    expect(readCommand("ls tools/ 2>/dev/null; df -h").effect).toBe("read");
  });

  it("names every pipe stage it knows", () => {
    expect(only("gh pr view 70 --json body | head -c 20000").notes).toEqual(["first 20000 bytes"]);
    expect(only("cat a.log | grep -v DEBUG").notes).toEqual(["without: DEBUG"]);
    expect(only("rg --files | wc -l").notes).toEqual(["counted lines"]);
    expect(only("cat a.json | jq .x").notes).toEqual(["parsed JSON"]);
    expect(only("ls | sort | uniq").notes).toEqual([]);
  });

  it("says when output is hidden or written", () => {
    expect(only("make >/dev/null 2>&1").notes).toEqual(["output hidden"]);
    expect(only("make &>/dev/null").notes).toEqual(["output hidden"]);
    const written = only("echo hi > out.txt");
    expect(written.notes).toEqual(["writes out.txt"]);
    expect(written.effect).toBe("write");
    expect(only("ls | tee listing.txt").effect).toBe("write");
  });

  it("folds cd into where the next steps happen", () => {
    const read = steps("cd app && bun run build && cd .. && ls");
    expect(read).toHaveLength(2);
    expect(read[0].notes).toEqual(["in app"]);
    expect(read[1].notes).toEqual(["in .."]);
  });

  it("folds `|| true` into the step it guards", () => {
    const read = steps("rm -rf dist || true");
    expect(read).toHaveLength(1);
    expect(read[0].notes).toEqual(["failure ignored"]);
  });

  it("names a sed range read", () => {
    const step = only("sed -n '175,365p' service/actions_chat.go");
    expect(`${step.done} ${step.target}`).toBe("Read actions_chat.go");
    expect(step.notes).toEqual(["lines 175 to 365"]);
    expect(step.verb).toBe("read");
  });
});

describe("effects", () => {
  it.each([
    ["cat a", "read"], ["ls", "read"], ["rg -n x src", "read"], ["git log -5", "read"], ["git status", "read"],
    ["gh pr view 70", "read"], ["gh api repos/a/b", "read"], ["df -h", "read"], ["pwd", "read"], ["jq .x a.json", "read"],
    ["sed -i '' 's/a/b/' f.ts", "write"], ["mkdir -p x", "write"], ["rm -rf x", "write"], ["mv a b", "write"],
    ["git checkout main", "write"], ["git commit -m x", "write"], ["git add .", "write"],
    ["python3 repro.py", "exec"], ["frobnicate --now", "exec"], ["curl https://x.dev", "exec"], ["git push", "exec"],
    ["gh pr create --title x", "exec"], ["gh api -X POST repos/a/b/issues", "exec"], ["gh api repos/a/b/issues -f title=x", "exec"],
    ["find . -name '*.tmp' -delete", "exec"], ["cat a | python3 -", "exec"],
  ])("%s is %s", (command, effect) => {
    expect(readCommand(command).effect).toBe(effect);
  });

  it("lets the strongest step win and leads with it", () => {
    const reading = readCommand("git status && git checkout fix && python repro.py");
    expect(reading.effect).toBe("exec");
    expect(leadStep(reading)?.target).toBe("python repro.py");
    expect(label("git status && git checkout fix && python repro.py")).toBe("Ran python repro.py + 2 more steps");
  });

  it("never calls an unknown program read-only", () => {
    expect(only("./scripts/mystery.sh").effect).toBe("exec");
  });
});

describe("nested commands", () => {
  it.each([
    [`echo "$(rm -rf build)"`, "exec"],
    ["echo `rm -rf build`", "exec"],
    ["cat <(curl https://x.dev)", "exec"],
    ["cat <(ls a) <(ls b)", "read"],
    ["ls $(pwd)", "read"],
    [`echo "$(cat a.txt | head -1)"`, "read"],
    ["echo 'literal $(rm -rf x)'", "read"],
    ["echo $((1 + 2))", "read"],
    ["echo $(rm -rf x", "exec"],
    ["cd $(mktemp -d) && ls", "exec"],
    ["X=$(rm -rf y)", "exec"],
    ["cat <<EOF\n$(rm -rf x)\nEOF", "exec"],
    ["cat <<'EOF'\n$(rm -rf x)\nEOF", "read"],
    ["GIT_EXTERNAL_DIFF=./x git diff", "exec"],
    ["LD_PRELOAD=./evil.so ls", "exec"],
    ["LANG=C ls", "read"],
  ])("%s is %s", (command, effect) => {
    expect(readCommand(command).effect).toBe(effect);
  });

  it("says a step runs a nested command, and keeps a cd that does as a step", () => {
    const read = steps("cd $(mktemp -d) && ls");
    expect(read).toHaveLength(2);
    expect(read[0].notes).toContain("runs a nested command");
    expect(read[1].effect).toBe("read");
  });

  it("finds substitutions the shell expands and no others", () => {
    expect(substitutions(`a "$(b)" \`c\` '$(d)' <(e) "<(f)" $((1+1))`)).toEqual(["b", "c", "e"]);
    expect(substitutions("$(a $(b) c)")).toEqual(["a $(b) c"]);
    expect(substitutions("$(open")).toBeNull();
  });
});

describe("phrases", () => {
  it.each([
    ["gh pr view 70 --json title,body", "Read PR #70"],
    ["gh pr view", "Read the current PR"],
    ["gh pr list --state open", "Listed pull requests"],
    ["gh pr diff 70", "Read PR #70 diff"],
    ["gh pr checks 70", "Checked CI on PR #70"],
    ["gh issue view 12", "Read issue #12"],
    ["gh run view 9", "Read workflow run 9"],
    ["gh api repos/x/y", "Called GitHub API repos/x/y"],
    ["df -h /p", "Checked disk space"],
    ["git checkout b", "Switched to b"],
    ["git checkout -b feat/x", "Switched to new branch feat/x"],
    ["git commit -m 'x'", "Committed changes"],
    ["git push origin main", "Pushed main"],
    ["git status", "Checked git status"],
    [".venv/bin/python ../vllm-hunt/repro_session_overlength.py --chunk2", "Ran python repro_session_overlength.py --chunk2"],
    ["/bin/zsh -lc \"rg -n 'workerRuntimes' src --glob '*.ts'\"", "Searched “workerRuntimes”"],
  ])("%s → %s", (command, expected) => {
    expect(label(command)).toBe(expected);
  });

  it("names a read-only chain by its first step and keeps its verb", () => {
    const phrase = commandPhrase(readCommand("git status && git diff"))!;
    expect(`${phrase.done} ${phrase.target}`).toBe("Checked git status + 1 more step");
    expect(phrase.verb).toBe("read");
  });

  it("marks git and gh steps with their program's glyph", () => {
    expect(only("git log").glyph).toBe("git");
    expect(only("gh pr view 1").glyph).toBe("github");
    expect(only("ls src").glyph).toBe("folder");
  });
});

describe("cache", () => {
  it("returns the same reading for the same command", () => {
    expect(readCommand("ls -la src")).toBe(readCommand("ls -la src"));
  });
});
