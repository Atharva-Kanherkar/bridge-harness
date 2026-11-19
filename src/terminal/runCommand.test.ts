import { describe, expect, it } from "vitest";
import { commandFromFence, isShellFence, terminalRunInput } from "./runCommand";

// Contract: testing/feat-run-in-terminal.md §Unit Tests.

describe("isShellFence", () => {
  it("accepts the shell languages in any case", () => {
    for (const lang of ["bash", "sh", "zsh", "shell", "console", "shell-session", "shellsession", "terminal", "BASH", " Zsh "]) {
      expect(isShellFence(lang)).toBe(true);
    }
  });

  it("rejects everything else", () => {
    for (const lang of ["", "ts", "python", "fish", "powershell", "text", "diff", "bashrc"]) {
      expect(isShellFence(lang)).toBe(false);
    }
  });
});

describe("commandFromFence", () => {
  it("runs a plain bash body as written", () => {
    expect(commandFromFence("bash", "bun install\nbun run dev")).toBe("bun install\nbun run dev");
  });

  it("returns null for a language a shell does not run", () => {
    expect(commandFromFence("ts", "const a = 1;")).toBeNull();
  });

  it("strips prompts from a bash transcript, keeps continuations, drops output", () => {
    const body = "$ docker run \\\n    -p 80:80 nginx\nUnable to find image\n$ docker ps";
    expect(commandFromFence("bash", body)).toBe("docker run \\\n    -p 80:80 nginx\ndocker ps");
  });

  it("does not treat a bash body as a transcript unless it starts with a prompt", () => {
    const body = "# install first\n$ not-a-prompt-here";
    expect(commandFromFence("bash", body)).toBe(body);
  });

  it("keeps only prompt lines and continuations in a console transcript", () => {
    const body = "Last login: today\n% echo hi \\\n> there\nhi there\n$ pwd\n/Users/me";
    expect(commandFromFence("console", body)).toBe("echo hi \\\nthere\npwd");
  });

  it("runs a console body without prompts whole", () => {
    expect(commandFromFence("console", "git status\ngit diff")).toBe("git status\ngit diff");
  });

  it("skips empty prompt lines", () => {
    expect(commandFromFence("console", "$\n$ ls")).toBe("ls");
  });

  it("strips control characters and blank edges", () => {
    expect(commandFromFence("sh", "\n\n  echo \u001b[31mred\u0007\u009b  \n\n")).toBe("  echo [31mred");
  });

  it("returns null when nothing is left to run", () => {
    expect(commandFromFence("bash", "\n  \n")).toBeNull();
    expect(commandFromFence("console", "$ \n$")).toBeNull();
  });
});

describe("terminalRunInput", () => {
  it("types each line and presses Enter without bracketed paste", () => {
    expect(terminalRunInput("cd app\nbun test", false)).toBe("cd app\rbun test\r");
  });

  it("wraps the command in one paste and presses Enter after it", () => {
    expect(terminalRunInput("cd app\nbun test", true)).toBe("\x1b[200~cd app\rbun test\x1b[201~\r");
  });

  it("cannot be broken out of a bracketed paste", () => {
    const input = terminalRunInput("echo a\x1b[201~; rm -rf ~", true);
    expect(input).toBe("\x1b[200~echo a[201~; rm -rf ~\x1b[201~\r");
    expect(input.split("\x1b[201~")).toHaveLength(2);
  });

  it("turns tabs into spaces outside a paste and keeps them inside one", () => {
    expect(terminalRunInput("if true; then\n\techo ok\nfi", false)).toBe("if true; then\r echo ok\rfi\r");
    expect(terminalRunInput("\techo ok", true)).toBe("\x1b[200~\techo ok\x1b[201~\r");
  });
});
