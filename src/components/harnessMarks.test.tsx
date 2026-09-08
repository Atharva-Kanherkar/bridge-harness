import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { HarnessMark, harnessTintClass } from "./harnessMarks";

const markup = (harness?: string | null, live = false) =>
  renderToStaticMarkup(<HarnessMark harness={harness} live={live} />);

describe("HarnessMark", () => {
  it("gives each built-in harness its own figure", () => {
    const drawn = ["claude", "codex", "opencode", "cursor"].map(harness => markup(harness));
    for (const [index, harness] of ["claude", "codex", "opencode", "cursor"].entries()) {
      expect(drawn[index]).toContain(`data-harness="${harness}"`);
    }
    // Four distinct figures, so no two harnesses read as the same agent.
    expect(new Set(drawn.map(html => html.replace(/data-harness="[^"]*"/, "")))).toHaveLength(4);
  });

  // The marks are the vendors' own paths now, not shapes Bridge generated from
  // an arm count. The old generator produced short two-point `M…L…` subpaths;
  // the real marks are long curves.
  it("draws the vendors' own marks rather than generated stand-ins", () => {
    expect(markup("codex")).toContain("M22.2819 9.8211");
    expect(markup("claude")).toContain("m4.7144 15.9555");
    expect(markup("opencode")).toContain("M2.4 0h19.2v24H2.4z");
    expect(markup("cursor")).toContain("12.00 3.40 19.45 7.70 4.55 7.70");
  });

  it("renders OpenAI in the page ink and never in a harness tint", () => {
    const codex = markup("codex");
    expect(codex).toContain("text-foreground");
    expect(codex).not.toContain("text-harness-");
    expect(harnessTintClass("codex")).toBe("text-foreground");
  });

  it("tints Claude and OpenCode, and drops OpenCode's inner block to 45%", () => {
    expect(markup("claude")).toContain("text-harness-claude");
    const opencode = markup("opencode");
    expect(opencode).toContain("text-harness-opencode");
    expect(opencode).toContain("opacity-45");
    expect(opencode).toContain("<rect");
  });

  // Cursor's five facets come from tokens so Paper can invert the ladder
  // without a second component.
  it("draws Cursor from its own facet tokens, with no Bridge hue", () => {
    const cursor = markup("cursor");
    expect(cursor.match(/<polygon/g)).toHaveLength(6);
    expect(cursor).toContain("--cursor-facet-lightest");
    expect(cursor).not.toContain("text-harness-");
  });

  // `bridge` means "Bridge picks the runtime", which is not an unknown agent.
  // Falling through to the gapped ring made every Bridge preset row look like it
  // was loading.
  it("gives Bridge its own achromatic glyph rather than the unknown spinner", () => {
    const bridge = markup("bridge");
    expect(bridge).toContain('data-harness="bridge"');
    expect(bridge).not.toContain("A8.6 8.6");
    expect(bridge).toContain("text-muted-foreground");
    expect(bridge).not.toContain("text-harness-");
  });

  // The harness id space is open, so an agent Bridge has no mark for must still
  // get something honest rather than another harness's figure.
  it("falls back to the arc spinner and the muted tint for an unknown harness", () => {
    const unknown = markup("some-acp-agent");
    expect(unknown).toContain("text-muted-foreground");
    expect(unknown).toContain("A8.6 8.6");
    expect(unknown).not.toContain("text-harness-");
    expect(markup(null)).toContain("A8.6 8.6");
    expect(harnessTintClass("some-acp-agent")).toBe("text-muted-foreground");
  });

  it("animates only when live, so reduced motion shows the frame we drew", () => {
    expect(markup("claude", true)).toContain("harness-mark-live");
    expect(markup("claude", false)).not.toContain("harness-mark-live");
  });

  // A vendor mark that spins reads as a logo being spun. Opacity is the only
  // channel `.harness-mark-live` is allowed to move.
  it("never rotates: the live class animates opacity alone", () => {
    const css = readFileSync(new URL("../index.css", import.meta.url), "utf8");
    const rule = /\.harness-mark-live\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
    expect(rule).toContain("harness-breathe");
    expect(rule).not.toContain("harness-turn");
    expect(css).not.toContain("@keyframes harness-turn");
  });

  // The label beside the mark always names the harness, so the mark itself is
  // decoration as far as assistive technology is concerned.
  it("is hidden from assistive technology", () => {
    for (const harness of ["claude", "codex", "opencode", "cursor", "unknown"]) {
      expect(markup(harness)).toContain('aria-hidden="true"');
    }
  });
});
