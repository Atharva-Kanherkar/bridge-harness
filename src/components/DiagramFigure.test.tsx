import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { DiagramFigure, isValidDiagramSpec, layoutDiagram, truncateLabel, type DiagramSpec } from "./DiagramFigure";

const BASIC: DiagramSpec = {
  nodes: [
    { id: "a", row: 0, col: 0, label: "start" },
    { id: "b", row: 1, col: 0 },
  ],
  edges: [{ from: "a", to: "b" }],
  caption: "A leads to B.",
  ariaLabel: "Diagram: A leads to B.",
};

describe("layoutDiagram", () => {
  it("places nodes on a fixed row/col grid, padded from the origin", () => {
    const { positions } = layoutDiagram(BASIC);
    expect(positions.a).toEqual({ x: 20, y: 20 });
    expect(positions.b).toEqual({ x: 20, y: 76 });
  });

  it("shifts negative columns so nothing renders off the left edge", () => {
    const spec: DiagramSpec = {
      nodes: [
        { id: "trunk", row: 0, col: 0 },
        { id: "branch", row: 1, col: -1 },
      ],
      edges: [{ from: "trunk", to: "branch", curve: true }],
      caption: "c",
      ariaLabel: "a",
    };
    const { positions } = layoutDiagram(spec);
    expect(positions.branch.x).toBeGreaterThanOrEqual(0);
    expect(positions.trunk.x).toBeGreaterThan(positions.branch.x);
  });

  it("computes out-degree so a fork point can be drawn heavier", () => {
    const spec: DiagramSpec = {
      nodes: [
        { id: "fork", row: 0, col: 0 },
        { id: "left", row: 1, col: -1 },
        { id: "right", row: 1, col: 0 },
      ],
      edges: [
        { from: "fork", to: "left", curve: true },
        { from: "fork", to: "right" },
      ],
      caption: "c",
      ariaLabel: "a",
    };
    const { outDegree } = layoutDiagram(spec);
    expect(outDegree.fork).toBe(2);
    expect(outDegree.left ?? 0).toBe(0);
  });

  it("reserves extra height only when a below-label or a continues marker is present", () => {
    const bare = layoutDiagram(BASIC);
    const withBelow = layoutDiagram({
      ...BASIC,
      nodes: [...BASIC.nodes, { id: "c", row: 1, col: -1, label: "aside", labelSide: "below" }],
    });
    const withContinues = layoutDiagram({
      ...BASIC,
      nodes: [BASIC.nodes[0], { ...BASIC.nodes[1], marker: "continues" }],
    });
    expect(withBelow.viewBox).not.toBe(bare.viewBox);
    expect(withContinues.viewBox).not.toBe(bare.viewBox);
  });

  it("widens column spacing so three adjacent labeled nodes never collide, on either side", () => {
    // The exact shape a live model produced: three labeled nodes one column
    // apart. A first fix (auto-flip to "below" on collision) turned out
    // insufficient here — it stopped the right-side collision but the two
    // "below" labels then collided with *each other* instead, since 1-column
    // spacing is too tight for either placement. The real fix is spacing:
    // column pitch widens to whatever the widest label needs, so the
    // collision this spec used to produce cannot happen in the first place.
    const spec: DiagramSpec = {
      nodes: [
        { id: "client", row: 0, col: 0, label: "Client (UI)" },
        { id: "server", row: 0, col: 1, label: "Server logic" },
        { id: "db", row: 0, col: 2, label: "Database" },
      ],
      edges: [
        { from: "client", to: "server" },
        { from: "server", to: "db" },
      ],
      caption: "c",
      ariaLabel: "a",
    };
    const { labelSides, positions } = layoutDiagram(spec);
    expect(labelSides.client).toBe("right");
    expect(labelSides.server).toBe("right");
    expect(labelSides.db).toBe("right");
    // Column pitch grew well past the 66px floor to fit "Server logic".
    expect(positions.server.x - positions.client.x).toBeGreaterThan(66);
  });

  it("does not flip to below when same-row nodes already have enough room", () => {
    const spec: DiagramSpec = {
      nodes: [
        { id: "a", row: 0, col: -2, label: "left" },
        { id: "b", row: 0, col: 0, label: "middle" },
        { id: "c", row: 0, col: 2, label: "right" },
      ],
      edges: [],
      caption: "c",
      ariaLabel: "a",
    };
    const { labelSides } = layoutDiagram(spec);
    expect(labelSides.a).toBe("right");
    expect(labelSides.b).toBe("right");
    expect(labelSides.c).toBe("right");
  });

  it("respects an explicit labelSide: below regardless of spacing", () => {
    const spec: DiagramSpec = {
      nodes: [
        { id: "a", row: 0, col: -2, label: "left", labelSide: "below" },
        { id: "b", row: 0, col: 0, label: "right neighbor" },
      ],
      edges: [],
      caption: "c",
      ariaLabel: "a",
    };
    const { labelSides } = layoutDiagram(spec);
    expect(labelSides.a).toBe("below");
  });
});

describe("truncateLabel", () => {
  it("leaves short labels untouched", () => {
    expect(truncateLabel("fork")).toBe("fork");
  });

  it("truncates an overly long label with an ellipsis", () => {
    const long = truncateLabel("a label that is much too long for a diagram node");
    expect(long.length).toBeLessThanOrEqual(18);
    expect(long.endsWith("…")).toBe(true);
  });
});

describe("isValidDiagramSpec", () => {
  it("accepts a well-formed spec", () => {
    expect(isValidDiagramSpec(BASIC)).toBe(true);
  });

  it.each([
    ["missing caption", { ...BASIC, caption: "" }],
    ["missing ariaLabel", { ...BASIC, ariaLabel: undefined }],
    ["empty nodes", { ...BASIC, nodes: [] }],
    ["duplicate node ids", { ...BASIC, nodes: [...BASIC.nodes, { id: "a", row: 2, col: 0 }] }],
    ["edge referencing an unknown node", { ...BASIC, edges: [{ from: "a", to: "ghost" }] }],
    ["bad emphasis enum", { ...BASIC, nodes: [{ ...BASIC.nodes[0], emphasis: "loud" }, BASIC.nodes[1]] }],
    ["bad marker enum", { ...BASIC, nodes: [{ ...BASIC.nodes[0], marker: "sparkle" }, BASIC.nodes[1]] }],
    ["not an object", "just a string"],
  ])("rejects: %s", (_label, value) => {
    expect(isValidDiagramSpec(value)).toBe(false);
  });
});

describe("DiagramFigure rendering", () => {
  it("renders an accessible, captioned SVG", () => {
    const html = renderToStaticMarkup(<DiagramFigure spec={BASIC} />);
    expect(html).toContain('role="img"');
    expect(html).toContain('aria-label="Diagram: A leads to B."');
    expect(html).toContain("A leads to B.");
    expect(html).toContain("start");
  });

  it("draws the accent color only on active-emphasis marks, never as the default", () => {
    const activeSpec: DiagramSpec = {
      nodes: [
        { id: "a", row: 0, col: 0 },
        { id: "b", row: 1, col: 0, emphasis: "active", marker: "tip" },
      ],
      edges: [{ from: "a", to: "b", emphasis: "active" }],
      caption: "c",
      ariaLabel: "a",
    };
    const neutralHtml = renderToStaticMarkup(<DiagramFigure spec={BASIC} />);
    const activeHtml = renderToStaticMarkup(<DiagramFigure spec={activeSpec} />);
    expect(neutralHtml).not.toContain("var(--ring)");
    expect(activeHtml).toContain("var(--ring)");
  });

  it("draws a halo for checkpoint and tip markers", () => {
    const spec: DiagramSpec = {
      nodes: [
        { id: "a", row: 0, col: 0, marker: "checkpoint" },
        { id: "b", row: 1, col: 0, marker: "tip" },
      ],
      edges: [{ from: "a", to: "b" }],
      caption: "c",
      ariaLabel: "a",
    };
    const html = renderToStaticMarkup(<DiagramFigure spec={spec} />);
    expect(html).toContain("stroke-dasharray");
  });
});
