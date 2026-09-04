import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { AgentEvent } from "../types";

/**
 * The boundary, at compile time.
 *
 * `@ts-expect-error` is the assertion: if comparing a wire kind to a literal
 * ever becomes legal again — someone re-brands `WireKind` as a string subtype,
 * say — this line stops erroring and `tsc` fails the build, which is the point.
 */
function comparingAWireKindToALiteralDoesNotCompile(event: AgentEvent): boolean {
  // @ts-expect-error a wire kind is opaque; open it with `readWireKind`.
  return event.kind === "tool.started";
}

/**
 * The boundary, enforced.
 *
 * Since #488 the transcript reads the wire in exactly one place. The type
 * system carries most of the weight — `AgentEvent.kind` is an opaque
 * `WireKind`, so a component cannot compare it to a literal without saying so
 * — but a type cannot express "and do not grow a per-harness branch either".
 * This test does.
 *
 * Two rules, over the components that draw a transcript:
 *
 * 1. No branch on harness identity. A row's shape comes from the normalized
 *    item, never from which agent produced it.
 * 2. No reading of wire event kinds. That is the codec's job, and it does it
 *    once, at ingestion.
 */

const read = (path: string) => readFileSync(fileURLToPath(new URL(path, import.meta.url)), "utf8");

/** Everything that turns a `ConversationItem` into pixels. */
const TRANSCRIPT_COMPONENTS = [
  "../components/AgentConversation.tsx",
  "../components/DiffView.tsx",
  "../components/TranscriptPane.tsx",
  "../components/workerPanel.ts",
  "../components/WorkerDetail.tsx",
];

/**
 * Where naming a harness is the point rather than a leak.
 *
 * `harnessMarks.tsx` draws each agent's figure and `harnessLabel` in
 * `utils.ts` spells its name; both are identity, which the transcript is
 * supposed to show. Neither decides how a row behaves, so neither is listed
 * above. `MissionControl.tsx` skips shell sessions in a worker roster, which is
 * a session-kind rule rather than a transcript one, and it draws no items.
 */
const IDENTITY_SITES = ["../components/harnessMarks.tsx", "../utils.ts"];

/**
 * Surfaces that legitimately watch the raw stream: the raw event inspector and
 * the worker activity feed both exist to show frames as frames. They are held
 * to rule 1 but not rule 2, and they must go through `readWireKind` — which is
 * what makes them greppable, and what this list is.
 */
const RAW_STREAM_SITES = ["../components/TranscriptPane.tsx", "../components/WorkerDetail.tsx"];

const HARNESS_BRANCH = /harness\s*[!=]==\s*["']/;

describe("harness branch gate", () => {
  it.each(TRANSCRIPT_COMPONENTS)("%s does not branch on harness identity", path => {
    const offending = read(path)
      .split("\n")
      .map((line, index) => [index + 1, line] as const)
      .filter(([, line]) => HARNESS_BRANCH.test(line));
    expect(offending, `add the behavior to the normalized item instead:\n${offending.map(([n, l]) => `${n}: ${l.trim()}`).join("\n")}`)
      .toEqual([]);
  });

  it.each(TRANSCRIPT_COMPONENTS.filter(path => !RAW_STREAM_SITES.includes(path)))(
    "%s does not read a wire event kind",
    path => {
      const source = read(path);
      // Comparing a `WireKind` to a literal is already a type error; what a
      // type cannot stop is opening one on purpose. These components have no
      // reason to. (Their own `kind` discriminants — a rendered row's kind, a
      // diff row's kind — are local unions and stay welcome.)
      expect(source).not.toContain("readWireKind");
      expect(source).not.toContain("asWireKind");
      expect(source).not.toMatch(/\bevent\.kind\b/);
    },
  );

  it.each(RAW_STREAM_SITES)("%s reads the wire only through the named hatch", path => {
    const source = read(path);
    // Every raw-kind read goes through `readWireKind(...)`, so `event.kind`
    // never appears bare. That is the whole guarantee: greppable, and typed.
    expect(source).not.toMatch(/(?<!readWireKind\()\bevent\.kind\b(?!\))/);
  });

  it("AgentConversation takes its shapes from the transcript layer, not from the wire", () => {
    const source = read("../components/AgentConversation.tsx");
    // It may hold raw events to hand to the projections; it may not open one.
    expect(source).toContain('from "../conversation"');
    expect(source).not.toContain('from "../transcript/wire"');
    expect(source).not.toContain('from "../transcript/codec"');
    // The reducers it calls are the public seam, not the removed internals.
    for (const removed of ["projectSessionEntry", "namedToolFacet", "reasoningDisplayText", "isLifecycleNoise"]) {
      expect(source, `${removed} is codec-internal now`).not.toContain(removed);
    }
  });

  it("names the identity sites it deliberately allows", () => {
    // Present so the exemption is a fact in the test rather than a silence: if
    // one of these disappears, this fails and the list gets revisited.
    for (const path of IDENTITY_SITES) {
      expect(read(path).length).toBeGreaterThan(0);
    }
    expect(read("../components/harnessMarks.tsx")).toMatch(/harness/i);
    expect(read("../utils.ts")).toContain("harnessLabel");
  });

  it("makes a wire-kind comparison a compile error, not a convention", () => {
    // The function above carries the assertion; calling it keeps it live.
    expect(typeof comparingAWireKindToALiteralDoesNotCompile).toBe("function");
  });

  it("keeps the codec the only module that opens a wire kind", () => {
    const codec = read("./codec.ts");
    expect(codec).toContain("readWireKind");
    // The reducer is downstream of it and must stay provider-blind.
    const reducer = read("./reducer.ts");
    expect(reducer).not.toContain("readWireKind");
    expect(reducer).not.toMatch(HARNESS_BRANCH);
    for (const provider of ["claude", "codex", "opencode", "cursor", "commandExecution", "tool_use"]) {
      expect(reducer.toLowerCase(), `${provider} belongs in the codec`).not.toContain(provider);
    }
  });
});
