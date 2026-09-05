import { readdirSync, readFileSync } from "node:fs";
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
 * above. Sites that do compare a harness id, for reasons that are not
 * transcript behavior, are in `HARNESS_BRANCH_ALLOWLIST` with their reasons.
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

/**
 * Everything that draws, over the whole UI rather than a list someone
 * remembered to extend.
 *
 * Rule 1 used to be checked against five named transcript files, which meant a
 * per-harness branch could walk back in through any component nobody had
 * thought of. It is checked here against every component plus `App.tsx`, and
 * the exceptions are a table with a reason each rather than an omission.
 */
function everyComponent(): string[] {
  const dir = fileURLToPath(new URL("../components/", import.meta.url));
  const files = readdirSync(dir, { recursive: true, encoding: "utf8" })
    .filter(name => name.endsWith(".tsx") && !name.includes(".test."))
    .map(name => `../components/${name}`)
    .sort();
  return [...files, "../App.tsx"];
}

/**
 * Where naming a harness is not a transcript behavior, with the reason it is
 * not. Anything on a rendering path that changes how a conversation item draws
 * belongs on the normalized item instead, and is never listed here.
 */
const HARNESS_BRANCH_ALLOWLIST: Record<string, string> = {
  "../components/MissionControl.tsx":
    "skips `shell` sessions in the worker roster: a session-kind rule, and the roster draws no conversation items",
  "../components/GitHubPane.tsx":
    "`bugbot` names a code-review provider on a pull request, not a harness that produces a transcript",
  "../components/settings/PresetsPage.tsx":
    "`bridge` is the preset-draft sentinel for 'Bridge chooses the runtime', in the form that picks one",
  "../App.tsx":
    "filters `shell` sessions out of the chat roster and labels a `bridge`-owned agent or slash command; neither reaches a transcript row",
};

describe("harness branch gate", () => {
  it.each(everyComponent())("%s does not branch on harness identity", path => {
    if (path in HARNESS_BRANCH_ALLOWLIST) return;
    const offending = read(path)
      .split("\n")
      .map((line, index) => [index + 1, line] as const)
      .filter(([, line]) => HARNESS_BRANCH.test(line));
    expect(offending, `add the behavior to the normalized item instead, or allowlist it with a reason:\n${offending.map(([n, l]) => `${n}: ${l.trim()}`).join("\n")}`)
      .toEqual([]);
  });

  it("keeps the allowlist honest", () => {
    // An entry that no longer branches is an entry that should be deleted: an
    // exemption nobody re-reads is how the next one gets waved through.
    for (const [path, reason] of Object.entries(HARNESS_BRANCH_ALLOWLIST)) {
      expect(read(path), `${path} no longer branches on a harness; drop it from the allowlist`)
        .toMatch(HARNESS_BRANCH);
      expect(reason.length, `${path} needs a reason`).toBeGreaterThan(20);
    }
    // And the transcript itself is never exempt.
    for (const path of TRANSCRIPT_COMPONENTS) {
      expect(HARNESS_BRANCH_ALLOWLIST[path], `${path} draws items and cannot be allowlisted`).toBeUndefined();
    }
  });

  it("watches the components the transcript is actually made of", () => {
    // The glob is the gate; this is the check that the glob found them.
    const watched = new Set(everyComponent());
    for (const path of TRANSCRIPT_COMPONENTS.filter(path => path.endsWith(".tsx"))) {
      expect(watched, `${path} fell out of the sweep`).toContain(path);
    }
    expect(watched.size).toBeGreaterThan(20);
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

  it("keeps one component in charge of the thinking presentation", () => {
    const source = read("../components/AgentConversation.tsx");
    // One mark, one call site for the class that animates it. Everything that
    // means "there is more of this coming" goes through `ThinkingMark`.
    const marks = source.split("\n").filter(line => line.includes("thinking-shimmer"));
    expect(marks, "the thinking sweep belongs to ThinkingMark and to nothing else").toHaveLength(1);
    expect(marks[0]).toContain("cn(");
    // And it is not decided by anything but the item's own status.
    expect(source).toContain("function Reasoning({ item }: { item: ConversationItem })");
    expect(source).toContain("const streaming = isStreamingText(item.status)");
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
