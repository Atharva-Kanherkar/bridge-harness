// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import specs from "../../../testing/fixtures/visualize/specs.json";
import type { VisualSpec } from "../spec";
import { VisualView } from "./VisualView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

interface Fixture {
  name: string;
  spec: VisualSpec;
  valid: boolean;
}

const goldens = (specs as Fixture[]).filter(fixture => fixture.valid);
const byName = (name: string) => goldens.find(fixture => fixture.name.startsWith(name))!.spec;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function draw(spec: VisualSpec, actions = {}) {
  act(() => root.render(<VisualView spec={spec} actions={actions} />));
}

describe("VisualView", () => {
  it.each(goldens.map(fixture => [fixture.name, fixture.spec] as const))("each_golden_renders_without_throwing: %s", (_, spec) => {
    draw(spec);
    for (const block of spec.blocks) {
      expect(container.querySelector(`[data-visual-block="${block.family}/${block.form}"]`), `${block.family}/${block.form}`).not.toBeNull();
    }
  });

  it("strings_render_as_text", () => {
    draw(byName("markup in strings"));
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("<img src=x onerror=alert(1)>");
    expect(container.innerHTML).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
  });

  it("estimate_values_are_marked", () => {
    draw(byName("line · solar"));
    // The 2025 row is an estimate: drawn hollow and labelled.
    const hollow = [...container.querySelectorAll("circle")].filter(circle => circle.getAttribute("fill") === "var(--card)");
    expect(hollow.length).toBeGreaterThan(0);
    expect(container.textContent).toContain("est.");
    expect(container.querySelector("path[stroke-dasharray]")).not.toBeNull();
  });

  it("colors_resolve_to_tokens_not_hex", () => {
    for (const fixture of goldens) {
      draw(fixture.spec);
      const markup = container.innerHTML;
      expect(markup, fixture.name).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    }
    draw(byName("stacked-bar"));
    const fills = new Set([...container.querySelectorAll("rect")].map(rect => rect.getAttribute("fill")));
    expect(fills).toContain("var(--chart-claude)");
    expect(fills).toContain("var(--chart-codex)");
  });

  it("findings_render_numbered_citations_matching_sources", () => {
    const spec = byName("line · solar");
    draw(spec);
    const findings = container.querySelector('[data-visual-block="document/findings"]')!;
    const first = findings.querySelector("li")!;
    // iea and irena are sources 1 and 2.
    expect([...first.querySelectorAll("span")].map(span => span.textContent)).toEqual(expect.arrayContaining(["1", "2"]));
    const sources = container.querySelector('[aria-label="Sources"]')!;
    expect(sources.textContent).toContain("iea.org");
    expect(sources.textContent).toContain("Estimate");
  });

  it("a follow-up chip asks, and a web source opens", () => {
    const ask = vi.fn();
    const open = vi.fn();
    draw(byName("stacked-bar"), { ask, open });
    const chip = [...container.querySelectorAll("button")].find(button => button.textContent === "Split Sep 30 by model")!;
    act(() => chip.click());
    expect(ask).toHaveBeenCalledWith("Split Sep 30 by model");
    draw(byName("line · solar"), { ask, open });
    const link = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("iea.org"))!;
    act(() => link.click());
    expect(open).toHaveBeenCalledWith("https://www.iea.org/reports/renewables-2024");
  });

  it("the title shows only when the host does not draw it", () => {
    const spec = byName("bar");
    draw(spec);
    expect(container.querySelector("h2")?.textContent).toBe(spec.title);
    act(() => root.render(<VisualView spec={spec} showTitle={false} />));
    expect(container.querySelector("h2")).toBeNull();
  });
});
