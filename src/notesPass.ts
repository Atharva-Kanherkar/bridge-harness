// Reading a Structure or Turn into tasks reply while it streams: the model
// writes one JSON object, and the page shows each field as soon as it can be
// read, before the object is complete.

import type { StructuredItem } from "./types";

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** The value a cut-off JSON text holds so far: every complete value, and the
 *  string being written, closed where it stops. Null when nothing is
 *  readable yet. */
export function partialJson(text: string): Json {
  const start = text.indexOf("{");
  if (start < 0) return null;
  const source = text.slice(start);
  const stack: string[] = [];
  /** Places the text can be cut and closed, with the containers open there. */
  const safe: { at: number; stack: string[] }[] = [];
  let inString = false;
  let stringIsValue = false;
  let escaped = false;
  let previous = "";
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    if (inString) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === "\"") {
        inString = false;
        if (stringIsValue) safe.push({ at: index + 1, stack: [...stack] });
        previous = "\"";
      }
      continue;
    }
    if (/\s/.test(char)) continue;
    if (char === "\"") {
      inString = true;
      stringIsValue = stack[stack.length - 1] === "[" || previous === ":";
    } else if (char === "{" || char === "[") {
      stack.push(char);
      safe.push({ at: index + 1, stack: [...stack] });
    } else if (char === "}" || char === "]") {
      if (previous !== "\"" && previous !== "}" && previous !== "]" && previous !== "{" && previous !== "[" && previous !== ",") safe.push({ at: index, stack: [...stack] });
      stack.pop();
      safe.push({ at: index + 1, stack: [...stack] });
      if (!stack.length) break;
    } else if (char === ",") {
      if (previous !== "\"" && previous !== "}" && previous !== "]") safe.push({ at: index, stack: [...stack] });
    }
    previous = char;
  }
  const close = (open: string[]) => open.slice().reverse().map(char => char === "{" ? "}" : "]").join("");
  const candidates: string[] = [];
  if (inString && stringIsValue) candidates.push(`${source.replace(/\\u[0-9a-fA-F]{0,3}$|\\$/, "")}"${close(stack)}`);
  for (let index = safe.length - 1; index >= 0 && candidates.length < 6; index -= 1) candidates.push(source.slice(0, safe[index].at) + close(safe[index].stack));
  for (const candidate of candidates) {
    try { return JSON.parse(candidate) as Json; } catch { /* try an earlier cut */ }
  }
  return null;
}

const text = (value: Json | undefined) => typeof value === "string" ? value : undefined;
const record = (value: Json | undefined): Record<string, Json> | null => value && typeof value === "object" && !Array.isArray(value) ? value : null;
const list = (value: Json | undefined): Json[] => Array.isArray(value) ? value : [];

export interface StreamingStructure {
  title?: string;
  kind?: string;
  tags: string[];
  summary?: string;
  items: StructuredItem[];
}

/** What a Structure reply says so far. */
export function streamingStructure(reply: string): StreamingStructure | null {
  const value = record(partialJson(reply));
  if (!value) return null;
  return {
    title: text(value.title),
    kind: text(value.kind),
    tags: list(value.tags).map(text).filter((tag): tag is string => !!tag),
    summary: text(value.summary),
    items: list(value.items).map(record).flatMap(item => {
      const title = text(item?.title);
      return item && title ? [{ title, excerpt: text(item.excerpt) ?? "", kind: text(item.kind) ?? "note" }] : [];
    }),
  };
}

/** The new task titles a Turn into tasks reply has so far. An entry naming an
 * existing task is a restatement the backend skips, so it is never a draft. */
export function streamingTasks(reply: string): { title: string; excerpt: string }[] {
  return list(record(partialJson(reply))?.tasks).map(record).flatMap(task => {
    const title = text(task?.title);
    return task && title && !text(task.existingTaskId)?.trim() ? [{ title, excerpt: text(task.excerpt) ?? "" }] : [];
  });
}
