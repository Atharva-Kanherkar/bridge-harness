/**
 * `VisualSpec` v1: the types, and a validator that mirrors
 * `mcp_apps::spec` in bridge-core rule for rule.
 *
 * The server is authoritative: it refuses anything invalid before the model's
 * turn moves on. This copy is the second line of defence, for the view and
 * the transcript, so a spec that somehow arrives malformed (a stale build,
 * another server that happens to be called `bridge`) is never drawn as if it
 * were valid. Both validators run `testing/fixtures/visualize/specs.json` and
 * must agree on every case.
 */

import { FAMILIES, availableIn, lookupForm, FORMS } from "./catalog";

export const MAX_SPEC_BYTES = 256 * 1024;
export const MAX_BLOCKS = 6;
export const MAX_ROWS = 5000;
export const MAX_SOURCES = 30;
export const MAX_DIAGRAM_NODES = 300;

export const COLORS = ["claude", "codex", "opencode", "cursor", "neutral", "accent", "positive", "negative"] as const;
export type VisualColor = (typeof COLORS)[number];

export type Scalar = string | number | boolean | null;
export type Row = Record<string, Scalar>;

export interface VisualSource {
  id: string;
  kind: "web" | "tool" | "file" | "user" | "computed" | "estimate";
  ref: string;
  title?: string;
  from?: string[];
}

export interface EncodingChannel {
  field?: string;
  type?: "quantitative" | "nominal" | "ordinal" | "temporal";
  title?: string;
  aggregate?: "sum" | "mean" | "average" | "count" | "min" | "max" | "median";
  sort?: unknown;
  axis?: { format?: string; title?: string } | null;
  value?: unknown;
}

export interface ChartBlock {
  family: "chart";
  form: string;
  title?: string;
  sourceIds: string[];
  colors?: Record<string, VisualColor>;
  vegaLite: {
    mark: string | { type: string; point?: boolean };
    data: { values: Row[] };
    encoding: Partial<Record<"x" | "y" | "color" | "theta" | "size" | "tooltip", EncodingChannel>>;
  };
}

export interface DocumentBlock {
  family: "document";
  form: string;
  title?: string;
  sourceIds?: string[];
  content: Record<string, unknown>;
}

export interface DiagramBlock {
  family: "diagram";
  form: string;
  title?: string;
  graph: {
    nodes: { id: string; label?: string; row: number; col: number; emphasis?: "default" | "muted" | "active"; marker?: "none" | "checkpoint" | "tip" | "continues"; labelSide?: "right" | "below" }[];
    edges: { from: string; to: string; curve?: boolean; emphasis?: "default" | "muted" | "active" }[];
    caption: string;
    ariaLabel: string;
  };
}

export type VisualBlock = ChartBlock | DocumentBlock | DiagramBlock;

export interface VisualSpec {
  version: 1;
  title: string;
  subtitle?: string;
  layout?: "stack" | "grid";
  blocks: VisualBlock[];
  sources?: VisualSource[];
  followUps?: string[];
  notes?: string[];
  redraw?: boolean;
}

export interface SpecError {
  path: string;
  message: string;
}

const SOURCE_KINDS = ["web", "tool", "file", "user", "computed", "estimate"];
const ENCODING_TYPES = ["quantitative", "nominal", "ordinal", "temporal"];
const AGGREGATES = ["sum", "mean", "average", "count", "min", "max", "median"];
const TOP_KEYS = ["version", "title", "subtitle", "layout", "blocks", "sources", "followUps", "notes", "redraw"];
const BLOCK_KEYS = ["family", "form", "title", "vegaLite", "colors", "sourceIds", "content", "graph"];
const FORBIDDEN_VEGA = ["url", "config", "usermeta", "href", "params", "selection", "signals", "transform", "datasets", "layer", "concat", "hconcat", "vconcat", "facet", "repeat", "projection"];
const FACT_DOCUMENTS = ["metric", "table", "compare"];

export function marksFor(form: string): string[] {
  switch (form) {
    case "bar": case "stacked-bar": case "grouped-bar": case "waterfall": case "funnel": return ["bar"];
    // The form decides the drawing: an arc spec still draws as a proportion bar.
    case "proportion": return ["bar", "arc"];
    case "line": return ["line"];
    case "area": return ["area"];
    case "scatter": return ["point", "circle"];
    case "heatmap": return ["rect"];
    default: return [];
  }
}

function requiredChannels(form: string): string[] {
  switch (form) {
    case "bar": case "line": case "area": case "scatter": case "waterfall": case "funnel": return ["x", "y"];
    case "stacked-bar": case "grouped-bar": case "heatmap": return ["x", "y", "color"];
    case "proportion": return ["color"];
    default: return [];
  }
}

type Json = unknown;
type JsonObject = Record<string, Json>;

const isObject = (value: Json): value is JsonObject => typeof value === "object" && value !== null && !Array.isArray(value);
const charCount = (text: string) => Array.from(text).length;
// Unicode Cc (C0, DEL, C1) except newline and tab, as Rust's `char::is_control`.
const CONTROL = /[\u0000-\u0008\u000B-\u001F\u007F-\u009F]/;
const join = (path: string, key: string) => (path ? `${path}.${key}` : key);
const index = (path: string, at: number) => `${path}[${at}]`;

class Check {
  errors: SpecError[] = [];

  push(path: string, message: string) {
    this.errors.push({ path, message });
  }

  text(value: Json, path: string, max: number, required: boolean): string | undefined {
    if (value === undefined || value === null) {
      if (required) this.push(path, "is required");
      return undefined;
    }
    if (typeof value !== "string") {
      this.push(path, "must be a string");
      return undefined;
    }
    if (required && value.trim() === "") {
      this.push(path, "must not be empty");
      return undefined;
    }
    if (charCount(value) > max) this.push(path, `is longer than ${max} characters`);
    if (CONTROL.test(value)) this.push(path, "contains control characters");
    return value;
  }

  boolean(value: Json, path: string) {
    if (value !== undefined && typeof value !== "boolean") this.push(path, "must be true or false");
  }

  oneOf(value: Json, path: string, allowed: readonly string[], required: boolean): string | undefined {
    const text = this.text(value, path, 64, required);
    if (text === undefined) return undefined;
    if (!allowed.includes(text)) {
      this.push(path, `must be one of: ${allowed.join(", ")}`);
      return undefined;
    }
    return text;
  }

  array(value: Json, path: string, min: number, max: number, required: boolean): Json[] | undefined {
    if (value === undefined || value === null) {
      if (required) this.push(path, "is required");
      return undefined;
    }
    if (!Array.isArray(value)) {
      this.push(path, "must be an array");
      return undefined;
    }
    if (value.length < min) this.push(path, `needs at least ${min} item${min === 1 ? "" : "s"}`);
    if (value.length > max) this.push(path, `has ${value.length} items; the limit is ${max}`);
    return value;
  }

  object(value: Json, path: string, required: boolean): JsonObject | undefined {
    if (value === undefined || value === null) {
      if (required) this.push(path, "is required");
      return undefined;
    }
    if (!isObject(value)) {
      this.push(path, "must be an object");
      return undefined;
    }
    return value;
  }

  knownKeys(map: JsonObject, path: string, allowed: readonly string[]) {
    for (const key of Object.keys(map)) {
      if (!allowed.includes(key)) this.push(join(path, key), `is not a known field; use: ${allowed.join(", ")}`);
    }
  }
}

/** Validate a spec. An empty list means valid. */
export function validateVisualSpec(spec: Json): SpecError[] {
  const check = new Check();
  let size: number;
  try {
    size = new TextEncoder().encode(JSON.stringify(spec) ?? "").length;
  } catch {
    size = Number.POSITIVE_INFINITY;
  }
  if (size > MAX_SPEC_BYTES) {
    check.push("", `is ${size} bytes; the limit is ${MAX_SPEC_BYTES}. Aggregate or sample the data`);
    return check.errors;
  }
  if (!isObject(spec)) {
    check.push("", "must be a JSON object");
    return check.errors;
  }
  check.knownKeys(spec, "", TOP_KEYS);
  if (spec.version !== 1) check.push("version", "must be 1");
  check.text(spec.title, "title", 80, true);
  check.text(spec.subtitle, "subtitle", 160, false);
  check.oneOf(spec.layout, "layout", ["stack", "grid"], false);
  check.boolean(spec.redraw, "redraw");
  check.array(spec.followUps, "followUps", 0, 4, false)?.forEach((item, at) => check.text(item, index("followUps", at), 80, true));
  check.array(spec.notes, "notes", 0, 6, false)?.forEach((item, at) => check.text(item, index("notes", at), 280, true));
  const sources = validateSources(check, spec.sources);
  check.array(spec.blocks, "blocks", 1, MAX_BLOCKS, true)?.forEach((block, at) => validateBlock(check, block, index("blocks", at), sources));
  return check.errors;
}

function validateSources(check: Check, value: Json): Map<string, string> {
  const kinds = new Map<string, string>();
  const items = check.array(value, "sources", 0, MAX_SOURCES, false);
  if (!items) return kinds;
  const pending: { path: string; own?: string; from: Json[] }[] = [];
  items.forEach((item, at) => {
    const path = index("sources", at);
    const source = check.object(item, path, true);
    if (!source) return;
    check.knownKeys(source, path, ["id", "kind", "ref", "title", "from"]);
    const id = check.text(source.id, join(path, "id"), 40, true);
    if (id !== undefined && !/^[A-Za-z0-9_-]*$/.test(id)) check.push(join(path, "id"), "may contain only letters, digits, - and _");
    const kind = check.oneOf(source.kind, join(path, "kind"), SOURCE_KINDS, true);
    const reference = check.text(source.ref, join(path, "ref"), 2000, true);
    check.text(source.title, join(path, "title"), 120, false);
    if (kind === "web" && reference !== undefined && !(reference.startsWith("https://") || reference.startsWith("http://"))) {
      check.push(join(path, "ref"), "must be the http(s) URL the value came from");
    }
    if (kind === "estimate" && reference !== undefined && charCount(reference.trim()) < 3) {
      check.push(join(path, "ref"), "must state the basis of the estimate");
    }
    if (kind === "computed") {
      const from = check.array(source.from, join(path, "from"), 1, MAX_SOURCES, true);
      if (from) pending.push({ path, own: id, from });
    }
    if (id !== undefined && kind !== undefined) {
      if (kinds.has(id)) check.push(join(path, "id"), `"${id}" is used by another source`);
      kinds.set(id, kind);
    }
  });
  for (const { path, own, from } of pending) {
    from.forEach((reference, at) => {
      const atPath = index(join(path, "from"), at);
      if (typeof reference !== "string") check.push(atPath, "must be a source id");
      else if (reference === own) check.push(atPath, "a computed source cannot read itself");
      else if (!kinds.has(reference)) check.push(atPath, `names "${reference}", which is not a declared source id`);
    });
  }
  return kinds;
}

function validateSourceIds(check: Check, value: Json, path: string, sources: Map<string, string>, required: boolean): string[] {
  // With exactly one declared source, a block that names none cites it.
  if (value === undefined && sources.size === 1) return [...sources.keys()];
  const items = check.array(value, path, required ? 1 : 0, MAX_SOURCES, required);
  if (!items) return [];
  const ids: string[] = [];
  items.forEach((item, at) => {
    if (typeof item !== "string") check.push(index(path, at), "must be a source id");
    else if (sources.has(item)) ids.push(item);
    else check.push(index(path, at), `names "${item}", which is not a declared source id`);
  });
  return ids;
}

function validateBlock(check: Check, value: Json, path: string, sources: Map<string, string>) {
  const block = check.object(value, path, true);
  if (!block) return;
  check.knownKeys(block, path, BLOCK_KEYS);
  check.text(block.title, join(path, "title"), 80, false);
  const family = check.oneOf(block.family, join(path, "family"), FAMILIES, true);
  if (!family) return;
  const form = check.text(block.form, join(path, "form"), 32, true);
  if (form === undefined) return;
  const info = lookupForm(family, form);
  if (!info) {
    const known = FORMS.filter(entry => entry.family === family).map(entry => entry.form);
    check.push(join(path, "form"), `"${form}" is not a ${family} form; ${family} forms are: ${known.join(", ")}`);
    return;
  }
  if (!info.available) {
    const available = availableIn(family);
    const hint = available.length ? `available ${family} forms: ${available.join(", ")}` : `no ${family} forms are available yet`;
    check.push(join(path, "form"), `"${form}" is not available yet; ${hint}`);
    return;
  }
  if (family === "chart") validateChart(check, block, path, form, sources);
  else if (family === "document") validateDocument(check, block, path, form, sources);
  else if (family === "diagram") validateGrid(check, block, path);
}

function forbidVegaFeatures(check: Check, value: Json, path: string) {
  if (Array.isArray(value)) {
    value.forEach((item, at) => forbidVegaFeatures(check, item, index(path, at)));
    return;
  }
  if (!isObject(value)) return;
  for (const [key, inner] of Object.entries(value)) {
    const at = join(path, key);
    if (FORBIDDEN_VEGA.includes(key)) {
      check.push(at, "is not supported");
      continue;
    }
    if (key === "values") continue;
    forbidVegaFeatures(check, inner, at);
  }
}

function validateChart(check: Check, block: JsonObject, path: string, form: string, sources: Map<string, string>) {
  const cited = validateSourceIds(check, block.sourceIds, join(path, "sourceIds"), sources, true);
  const colors = check.object(block.colors, join(path, "colors"), false);
  if (colors) for (const [series, color] of Object.entries(colors)) check.oneOf(color, join(join(path, "colors"), series), COLORS, true);
  const vegaPath = join(path, "vegaLite");
  const vega = check.object(block.vegaLite, vegaPath, true);
  if (!vega) return;
  forbidVegaFeatures(check, vega, vegaPath);

  const markPath = join(vegaPath, "mark");
  const mark = typeof vega.mark === "string" ? vega.mark : isObject(vega.mark) && typeof vega.mark.type === "string" ? vega.mark.type : undefined;
  const allowed = marksFor(form);
  if (mark === undefined) check.push(markPath, `is required; a ${form} uses mark "${allowed.join('" or "')}"`);
  else if (!allowed.includes(mark)) check.push(markPath, `"${mark}" does not draw a ${form}; use "${allowed.join('" or "')}"`);

  const dataPath = join(vegaPath, "data");
  const data = check.object(vega.data, dataPath, true);
  let rows: Json[] | undefined;
  if (data) {
    if ("name" in data) check.push(join(dataPath, "name"), "pass the rows inline in data.values");
    rows = check.array(data.values, join(dataPath, "values"), 1, MAX_ROWS, true);
  }
  const fields = new Set<string>();
  let estimates = false;
  rows?.forEach((row, at) => {
    const rowPath = index(join(dataPath, "values"), at);
    if (!isObject(row)) {
      check.push(rowPath, "must be an object of field: value");
      return;
    }
    for (const [key, cell] of Object.entries(row)) {
      fields.add(key);
      if (typeof cell === "string") {
        if (charCount(cell) > 200) check.push(join(rowPath, key), "is longer than 200 characters");
        else if (CONTROL.test(cell)) check.push(join(rowPath, key), "contains control characters");
      } else if (typeof cell === "number") {
        if (!Number.isFinite(cell)) check.push(join(rowPath, key), "must be a string, number, true/false or null");
      } else if (typeof cell === "boolean") {
        if (key === "estimate" && cell) estimates = true;
      } else if (cell !== null) {
        check.push(join(rowPath, key), "must be a string, number, true/false or null");
      }
    }
  });
  if (estimates && !cited.some(id => sources.get(id) === "estimate")) {
    check.push(join(path, "sourceIds"), "rows marked estimate: true need an estimate source stating the basis");
  }

  const encodingPath = join(vegaPath, "encoding");
  const encoding = check.object(vega.encoding, encodingPath, true);
  if (!encoding) return;
  for (const channel of requiredChannels(form)) {
    let present = channel in encoding;
    if (form === "proportion" && channel === "color") {
      present = present && ("x" in encoding || "theta" in encoding);
      if (!present) {
        check.push(encodingPath, "a proportion needs color (the parts) and x or theta (their size)");
        continue;
      }
    }
    if (!present) check.push(join(encodingPath, channel), `is required for a ${form}`);
  }
  for (const [channel, definition] of Object.entries(encoding)) {
    const channelPath = join(encodingPath, channel);
    if (!isObject(definition)) {
      if (channel === "tooltip" && Array.isArray(definition)) continue;
      check.push(channelPath, 'must be an object like {"field": "…", "type": "…"}');
      continue;
    }
    if ("value" in definition && channel === "color") check.oneOf(definition.value, join(channelPath, "value"), COLORS, true);
    if ("type" in definition) check.oneOf(definition.type, join(channelPath, "type"), ENCODING_TYPES, true);
    if ("aggregate" in definition) check.oneOf(definition.aggregate, join(channelPath, "aggregate"), AGGREGATES, true);
    if (isObject(definition.scale)) {
      for (const key of ["range", "scheme"]) {
        if (key in definition.scale) check.push(join(join(channelPath, "scale"), key), "colours come from the block's colors map, not the scale");
      }
    }
    if (typeof definition.field === "string") {
      if (fields.size > 0 && !fields.has(definition.field)) check.push(join(channelPath, "field"), `"${definition.field}" is not a field of any row in data.values`);
    } else if (definition.field !== undefined) {
      check.push(join(channelPath, "field"), "must be a field name");
    } else if (!(definition.aggregate === "count" || "value" in definition || channel === "tooltip")) {
      check.push(join(channelPath, "field"), "is required");
    }
  }
}

function scalarCell(check: Check, value: Json, path: string, max: number) {
  if (typeof value === "string") check.text(value, path, max, false);
  else if (!(typeof value === "number" || typeof value === "boolean" || value === null)) check.push(path, "must be a string, number, true/false or null");
}

function items(check: Check, content: JsonObject, contentPath: string, min: number, max: number, keys: string[], each: (item: JsonObject, path: string) => void) {
  check.knownKeys(content, contentPath, ["items"]);
  const itemsPath = join(contentPath, "items");
  check.array(content.items, itemsPath, min, max, true)?.forEach((value, at) => {
    const atPath = index(itemsPath, at);
    const item = check.object(value, atPath, true);
    if (!item) return;
    check.knownKeys(item, atPath, keys);
    each(item, atPath);
  });
}

function validateDocument(check: Check, block: JsonObject, path: string, form: string, sources: Map<string, string>) {
  const cited = validateSourceIds(check, block.sourceIds, join(path, "sourceIds"), sources, FACT_DOCUMENTS.includes(form));
  const contentPath = join(path, "content");
  const content = check.object(block.content, contentPath, true);
  if (!content) return;
  let estimates = false;
  switch (form) {
    case "metric":
      items(check, content, contentPath, 1, 6, ["label", "value", "delta", "tone", "note", "estimate"], (item, at) => {
        check.text(item.label, join(at, "label"), 60, true);
        if (typeof item.value !== "number") check.text(item.value, join(at, "value"), 24, true);
        check.text(item.delta, join(at, "delta"), 24, false);
        check.oneOf(item.tone, join(at, "tone"), ["positive", "negative", "neutral"], false);
        check.text(item.note, join(at, "note"), 120, false);
        check.boolean(item.estimate, join(at, "estimate"));
        if (item.estimate === true) estimates = true;
      });
      break;
    case "cards":
      items(check, content, contentPath, 1, 12, ["title", "body", "meta", "tags"], (item, at) => {
        check.text(item.title, join(at, "title"), 80, true);
        check.text(item.body, join(at, "body"), 400, false);
        check.text(item.meta, join(at, "meta"), 80, false);
        check.array(item.tags, join(at, "tags"), 0, 6, false)?.forEach((tag, tagAt) => check.text(tag, index(join(at, "tags"), tagAt), 24, true));
      });
      break;
    case "compare": {
      check.knownKeys(content, contentPath, ["criteria", "options"]);
      const criteriaPath = join(contentPath, "criteria");
      const criteria = (check.array(content.criteria, criteriaPath, 1, 12, true) ?? [])
        .map((item, at) => check.text(item, index(criteriaPath, at), 60, true))
        .filter((item): item is string => item !== undefined);
      const optionsPath = join(contentPath, "options");
      check.array(content.options, optionsPath, 2, 5, true)?.forEach((value, at) => {
        const atPath = index(optionsPath, at);
        const option = check.object(value, atPath, true);
        if (!option) return;
        check.knownKeys(option, atPath, ["name", "highlight", "summary", "values"]);
        check.text(option.name, join(atPath, "name"), 40, true);
        check.boolean(option.highlight, join(atPath, "highlight"));
        check.text(option.summary, join(atPath, "summary"), 160, false);
        const values = check.object(option.values, join(atPath, "values"), true);
        if (values) {
          for (const [criterion, cell] of Object.entries(values)) {
            const cellPath = join(join(atPath, "values"), criterion);
            if (criteria.length > 0 && !criteria.includes(criterion)) check.push(cellPath, "is not one of content.criteria");
            scalarCell(check, cell, cellPath, 80);
          }
        }
      });
      break;
    }
    case "table": {
      check.knownKeys(content, contentPath, ["columns", "rows"]);
      const columnsPath = join(contentPath, "columns");
      const keys: string[] = [];
      check.array(content.columns, columnsPath, 1, 8, true)?.forEach((value, at) => {
        const atPath = index(columnsPath, at);
        const column = check.object(value, atPath, true);
        if (!column) return;
        check.knownKeys(column, atPath, ["key", "label", "align", "format"]);
        const key = check.text(column.key, join(atPath, "key"), 40, true);
        if (key !== undefined) keys.push(key);
        check.text(column.label, join(atPath, "label"), 40, true);
        check.oneOf(column.align, join(atPath, "align"), ["left", "right", "center"], false);
        check.oneOf(column.format, join(atPath, "format"), ["text", "number", "percent", "currency", "compact"], false);
      });
      const rowsPath = join(contentPath, "rows");
      check.array(content.rows, rowsPath, 1, 200, true)?.forEach((value, at) => {
        const atPath = index(rowsPath, at);
        const row = check.object(value, atPath, true);
        if (!row) return;
        for (const [key, cell] of Object.entries(row)) {
          if (key === "estimate") {
            check.boolean(cell, join(atPath, key));
            if (cell === true) estimates = true;
            continue;
          }
          if (keys.length > 0 && !keys.includes(key)) check.push(join(atPath, key), "is not a column key");
          scalarCell(check, cell, join(atPath, key), 200);
        }
      });
      break;
    }
    case "callout":
      check.knownKeys(content, contentPath, ["tone", "title", "body"]);
      check.oneOf(content.tone, join(contentPath, "tone"), ["info", "warning", "success", "danger", "neutral"], true);
      check.text(content.title, join(contentPath, "title"), 80, false);
      check.text(content.body, join(contentPath, "body"), 600, true);
      break;
    case "steps":
      items(check, content, contentPath, 2, 12, ["title", "body"], (item, at) => {
        check.text(item.title, join(at, "title"), 80, true);
        check.text(item.body, join(at, "body"), 400, false);
      });
      break;
    case "checklist":
      items(check, content, contentPath, 1, 20, ["text", "done"], (item, at) => {
        check.text(item.text, join(at, "text"), 160, true);
        if (typeof item.done !== "boolean") check.push(join(at, "done"), "must be true or false");
      });
      break;
    case "findings":
      items(check, content, contentPath, 1, 8, ["text", "sourceIds", "estimate"], (item, at) => {
        check.text(item.text, join(at, "text"), 280, true);
        const ids = validateSourceIds(check, item.sourceIds, join(at, "sourceIds"), sources, true);
        check.boolean(item.estimate, join(at, "estimate"));
        if (item.estimate === true && !ids.some(id => sources.get(id) === "estimate")) {
          check.push(join(at, "sourceIds"), "an estimate finding needs an estimate source stating the basis");
        }
      });
      break;
    case "pros-cons":
      check.knownKeys(content, contentPath, ["pros", "cons"]);
      for (const side of ["pros", "cons"]) {
        const sidePath = join(contentPath, side);
        check.array(content[side], sidePath, 1, 8, true)?.forEach((item, at) => check.text(item, index(sidePath, at), 160, true));
      }
      break;
    case "glossary":
      items(check, content, contentPath, 1, 20, ["term", "definition"], (item, at) => {
        check.text(item.term, join(at, "term"), 40, true);
        check.text(item.definition, join(at, "definition"), 280, true);
      });
      break;
  }
  if (estimates && !cited.some(id => sources.get(id) === "estimate")) {
    check.push(join(path, "sourceIds"), "values marked estimate need an estimate source stating the basis");
  }
}

function validateGrid(check: Check, block: JsonObject, path: string) {
  const graphPath = join(path, "graph");
  const graph = check.object(block.graph, graphPath, true);
  if (!graph) return;
  check.knownKeys(graph, graphPath, ["nodes", "edges", "caption", "ariaLabel"]);
  check.text(graph.caption, join(graphPath, "caption"), 200, true);
  check.text(graph.ariaLabel, join(graphPath, "ariaLabel"), 200, true);
  const emphasis = ["default", "muted", "active"];
  const ids = new Set<string>();
  const nodesPath = join(graphPath, "nodes");
  check.array(graph.nodes, nodesPath, 1, MAX_DIAGRAM_NODES, true)?.forEach((value, at) => {
    const atPath = index(nodesPath, at);
    const node = check.object(value, atPath, true);
    if (!node) return;
    check.knownKeys(node, atPath, ["id", "label", "row", "col", "emphasis", "marker", "labelSide"]);
    const id = check.text(node.id, join(atPath, "id"), 40, true);
    if (id !== undefined) {
      if (ids.has(id)) check.push(join(atPath, "id"), `"${id}" is used by another node`);
      ids.add(id);
    }
    check.text(node.label, join(atPath, "label"), 60, false);
    for (const axis of ["row", "col"]) {
      const cell = node[axis];
      if (!(typeof cell === "number" && Number.isInteger(cell) && cell >= 0 && cell <= 60)) check.push(join(atPath, axis), "must be a whole number from 0 to 60");
    }
    check.oneOf(node.emphasis, join(atPath, "emphasis"), emphasis, false);
    check.oneOf(node.marker, join(atPath, "marker"), ["none", "checkpoint", "tip", "continues"], false);
    check.oneOf(node.labelSide, join(atPath, "labelSide"), ["right", "below"], false);
  });
  const edgesPath = join(graphPath, "edges");
  check.array(graph.edges, edgesPath, 0, MAX_DIAGRAM_NODES * 3, true)?.forEach((value, at) => {
    const atPath = index(edgesPath, at);
    const edge = check.object(value, atPath, true);
    if (!edge) return;
    check.knownKeys(edge, atPath, ["from", "to", "curve", "emphasis"]);
    for (const end of ["from", "to"]) {
      const id = check.text(edge[end], join(atPath, end), 40, true);
      if (id !== undefined && !ids.has(id)) check.push(join(atPath, end), `names "${id}", which is not a node id`);
    }
    check.boolean(edge.curve, join(atPath, "curve"));
    check.oneOf(edge.emphasis, join(atPath, "emphasis"), emphasis, false);
  });
}

export type ParsedSpec = { ok: true; spec: VisualSpec } | { ok: false; errors: SpecError[] };

/** Validate and narrow. A spec that fails is never drawn. */
export function parseVisualSpec(input: unknown): ParsedSpec {
  const errors = validateVisualSpec(input);
  return errors.length ? { ok: false, errors } : { ok: true, spec: input as VisualSpec };
}
