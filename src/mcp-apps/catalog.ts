/**
 * Every visual form a VisualSpec can name, and which ones this build draws.
 * Mirrors `mcp_apps::catalog` in bridge-core; both are checked against
 * `testing/fixtures/visualize/forms.json`.
 */
export const FAMILIES = ["chart", "diagram", "map", "time", "math", "document"] as const;
export type Family = (typeof FAMILIES)[number];

export interface FormInfo {
  family: Family;
  form: string;
  available: boolean;
}

export const FORMS: readonly FormInfo[] = [
  { family: "chart", form: "bar", available: true },
  { family: "chart", form: "stacked-bar", available: true },
  { family: "chart", form: "grouped-bar", available: true },
  { family: "chart", form: "line", available: true },
  { family: "chart", form: "area", available: true },
  { family: "chart", form: "scatter", available: true },
  { family: "chart", form: "heatmap", available: true },
  { family: "chart", form: "proportion", available: true },
  { family: "chart", form: "waterfall", available: true },
  { family: "chart", form: "funnel", available: true },
  { family: "chart", form: "bubble", available: false },
  { family: "chart", form: "histogram", available: false },
  { family: "chart", form: "box", available: false },
  { family: "chart", form: "treemap", available: false },
  { family: "chart", form: "sunburst", available: false },
  { family: "chart", form: "sankey", available: false },
  { family: "chart", form: "slope", available: false },
  { family: "chart", form: "dumbbell", available: false },
  { family: "chart", form: "candlestick", available: false },
  { family: "chart", form: "sparkline-grid", available: false },
  { family: "diagram", form: "grid", available: true },
  { family: "diagram", form: "flow", available: false },
  { family: "diagram", form: "tree", available: false },
  { family: "diagram", form: "org", available: false },
  { family: "diagram", form: "mindmap", available: false },
  { family: "diagram", form: "sequence", available: false },
  { family: "diagram", form: "state", available: false },
  { family: "diagram", form: "er", available: false },
  { family: "diagram", form: "dependency", available: false },
  { family: "diagram", form: "architecture", available: false },
  { family: "diagram", form: "venn", available: false },
  { family: "diagram", form: "quadrant", available: false },
  { family: "map", form: "choropleth", available: false },
  { family: "map", form: "points", available: false },
  { family: "map", form: "hex", available: false },
  { family: "time", form: "timeline", available: false },
  { family: "time", form: "lanes", available: false },
  { family: "time", form: "week", available: false },
  { family: "time", form: "month", available: false },
  { family: "math", form: "equation", available: false },
  { family: "math", form: "plot", available: false },
  { family: "math", form: "matrix", available: false },
  { family: "math", form: "vectors", available: false },
  { family: "document", form: "metric", available: true },
  { family: "document", form: "cards", available: true },
  { family: "document", form: "compare", available: true },
  { family: "document", form: "table", available: true },
  { family: "document", form: "callout", available: true },
  { family: "document", form: "steps", available: true },
  { family: "document", form: "checklist", available: true },
  { family: "document", form: "findings", available: true },
  { family: "document", form: "pros-cons", available: true },
  { family: "document", form: "glossary", available: true },
  { family: "document", form: "gallery", available: false },
];

export function lookupForm(family: string, form: string): FormInfo | undefined {
  return FORMS.find(info => info.family === family && info.form === form);
}

export function availableIn(family: string): string[] {
  return FORMS.filter(info => info.family === family && info.available).map(info => info.form);
}
