import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { HarnessMark, harnessTintClass, hasHarnessMark } from "./harnessMarks";

const markup = (harness?: string | null, live = false) =>
  renderToStaticMarkup(<HarnessMark harness={harness} live={live} />);

describe("HarnessMark", () => {
  it("gives each built-in harness its own figure and its own tint", () => {
    const claude = markup("claude");
    const codex = markup("codex");
    const opencode = markup("opencode");
    expect(claude).toContain("text-harness-claude");
    expect(codex).toContain("text-harness-codex");
    expect(opencode).toContain("text-harness-opencode");
    // Three distinct paths, so no two harnesses read as the same agent.
    const paths = [claude, codex, opencode].map(html => /d="([^"]+)"/.exec(html)?.[1]);
    expect(new Set(paths).size).toBe(3);
  });

  it("only OpenCode's mark carries the solid core", () => {
    expect(markup("opencode")).toContain("<circle");
    expect(markup("claude")).not.toContain("<circle");
    expect(markup("codex")).not.toContain("<circle");
  });

  // The harness id space is open, so an agent Bridge has no mark for must still
  // get something honest rather than another harness's figure.
  it("falls back to the arc spinner and the muted tint for an unknown harness", () => {
    const unknown = markup("some-acp-agent");
    expect(unknown).toContain("text-muted-foreground");
    expect(unknown).toContain("A8.6 8.6");
    expect(unknown).not.toContain("text-harness-");
    expect(markup(null)).toContain("A8.6 8.6");
    expect(hasHarnessMark("some-acp-agent")).toBe(false);
    expect(hasHarnessMark("claude")).toBe(true);
    expect(harnessTintClass("some-acp-agent")).toBe("text-muted-foreground");
  });

  it("animates only when live, so reduced motion shows the frame we drew", () => {
    expect(markup("claude", true)).toContain("harness-mark-live");
    expect(markup("claude", false)).not.toContain("harness-mark-live");
  });

  // The label beside the mark always names the harness, so the mark itself is
  // decoration as far as assistive technology is concerned.
  it("is hidden from assistive technology", () => {
    for (const harness of ["claude", "codex", "opencode", "unknown"]) {
      expect(markup(harness)).toContain('aria-hidden="true"');
    }
  });
});
