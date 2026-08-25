import { useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import { startSerialPoll } from "./polling";
import type { ContextBreakdownResult, ContextBreakdownSegment } from "./protocol/generated/protocol";

export type BreakdownGroup = "conversation" | "prompt" | "adapter";

export interface SegmentClassMeta {
  label: string;
  group: BreakdownGroup;
  editable?: boolean;
}

const SEGMENT_CLASS_META: Record<string, SegmentClassMeta> = {
  conversation: { label: "Conversation", group: "conversation" },
  "prompt-stable": { label: "Bridge sections · stable", group: "prompt", editable: true },
  "prompt-variable": { label: "Bridge sections · variable", group: "prompt", editable: true },
  providerBaseInstructions: { label: "Provider base instructions", group: "adapter" },
  toolSchemas: { label: "Tool schemas", group: "adapter" },
  mcpDynamicTools: { label: "MCP & dynamic tools", group: "adapter" },
  skillsPlugins: { label: "Skills & plugins", group: "adapter" },
  agentDefinitions: { label: "Agent definitions", group: "adapter" },
};

export function segmentClassMeta(segmentClass: string): SegmentClassMeta {
  const known = SEGMENT_CLASS_META[segmentClass];
  if (known) return known;
  const spaced = segmentClass.replace(/([a-z])([A-Z])/g, "$1 $2").replace(/[-_]/g, " ");
  const label = spaced.charAt(0).toUpperCase() + spaced.slice(1);
  return { label, group: "adapter" };
}

export interface RankedSegment {
  index: number;
  segment: ContextBreakdownSegment;
  meta: SegmentClassMeta;
}

/** Available segments by size desc, unsized available next, unavailable last; stable within ties. */
export function rankSegments(segments: ContextBreakdownSegment[]): RankedSegment[] {
  const bucket = (segment: ContextBreakdownSegment): 0 | 1 | 2 => {
    if (segment.state === "unavailable") return 2;
    return segment.tokens != null || segment.bytes != null ? 0 : 1;
  };
  const size = (segment: ContextBreakdownSegment): number => segment.tokens ?? segment.bytes ?? 0;
  return segments
    .map((segment, index) => ({ index, segment, meta: segmentClassMeta(segment.segmentClass) }))
    .sort((a, b) => {
      const difference = bucket(a.segment) - bucket(b.segment);
      if (difference !== 0) return difference;
      if (bucket(a.segment) === 0) {
        const bySize = size(b.segment) - size(a.segment);
        if (bySize !== 0) return bySize;
      }
      return a.index - b.index;
    });
}

export interface BreakdownMath {
  knownTokens: number;
  occupiedTokens: number;
  windowTokens: number;
  unattributedTokens: number;
  freeTokens: number;
}

export function breakdownMath(result: ContextBreakdownResult): BreakdownMath {
  const knownTokens = result.segments.reduce(
    (sum, segment) => segment.state !== "unavailable" && segment.tokens != null ? sum + segment.tokens : sum,
    0,
  );
  const occupiedTokens = Math.max(0, result.conversation.tokenEstimate);
  const windowTokens = Math.max(0, result.conversation.contextWindowTokens);
  return {
    knownTokens,
    occupiedTokens,
    windowTokens,
    unattributedTokens: Math.max(0, occupiedTokens - knownTokens),
    freeTokens: Math.max(0, windowTokens - occupiedTokens),
  };
}

export function formatTokens(value: number): string {
  return Math.round(value).toLocaleString("en-US");
}

export function formatCompactTokens(value: number): string {
  if (value < 10_000) return formatTokens(value);
  const thousands = value / 1000;
  const rounded = thousands >= 100 ? Math.round(thousands) : Math.round(thousands * 10) / 10;
  return `${rounded}k`;
}

export interface DeltaSummary {
  text: string;
  detail: string | null;
}

export function deltaSummary(result: ContextBreakdownResult): DeltaSummary | null {
  const delta = result.compactionDelta;
  if (!delta) return null;
  const sign = delta.growthTokens >= 0 ? "+" : "−";
  const text = `${sign}${formatTokens(Math.abs(delta.growthTokens))} tok`;
  const details = [delta.reason?.trim(), delta.sourceAgent.trim()].filter(Boolean);
  return { text, detail: details.length ? details.join(" · ") : null };
}

const MISSING_METHOD_PATTERN = /not found|unknown command|no such command|unimplemented/i;
const MAX_CONSECUTIVE_FAILURES = 3;

export interface BreakdownFetchers {
  digest: (sessionId: string) => Promise<string>;
  fetch: (sessionId: string) => Promise<ContextBreakdownResult>;
}

export const defaultBreakdownFetchers: BreakdownFetchers = {
  digest: sessionId => bridgeApi.contextBreakdownDigest(sessionId),
  fetch: sessionId => bridgeApi.contextBreakdown(sessionId),
};

export interface ContextBreakdownState {
  result: ContextBreakdownResult | null;
  reconciledAt: string | null;
  unavailable: boolean;
}

type TimerHandle = ReturnType<typeof setTimeout>;

export interface PollScheduler {
  schedule: (callback: () => void, delayMs: number) => TimerHandle;
  cancel: (handle: TimerHandle) => void;
}

const defaultPollScheduler: PollScheduler = {
  schedule: (callback, delayMs) => setTimeout(callback, delayMs),
  cancel: handle => clearTimeout(handle),
};

/**
 * Serial, bounded, digest-gated reconciliation for one session's breakdown.
 * The full payload is fetched only when the opaque digest token changes;
 * failures never overlap (startSerialPoll) and give up after repeated misses
 * so an absent backend cannot be hammered. Changing the session discards all
 * prior state before the new loop starts.
 */
export function useContextBreakdown(
  sessionId: string | null | undefined,
  enabled: boolean,
  fetchers: BreakdownFetchers = defaultBreakdownFetchers,
  intervalMs = 15_000,
  scheduler: PollScheduler = defaultPollScheduler,
): ContextBreakdownState {
  const [state, setState] = useState<ContextBreakdownState>({ result: null, reconciledAt: null, unavailable: false });
  const digestRef = useRef<string | null>(null);
  const failuresRef = useRef(0);

  useEffect(() => {
    digestRef.current = null;
    failuresRef.current = 0;
    setState({ result: null, reconciledAt: null, unavailable: false });
    if (!enabled || !sessionId) return;

    let dead = false;
    let cancel: (() => void) | undefined;

    cancel = startSerialPoll(async () => {
      try {
        const digest = await fetchers.digest(sessionId);
        if (dead) return;
        failuresRef.current = 0;
        if (digest === digestRef.current) return;
        const result = await fetchers.fetch(sessionId);
        if (dead) return;
        digestRef.current = digest;
        setState({ result, reconciledAt: new Date().toISOString(), unavailable: false });
      } catch (error) {
        if (dead) return;
        const message = error instanceof Error ? error.message : String(error);
        if (MISSING_METHOD_PATTERN.test(message) || ++failuresRef.current >= MAX_CONSECUTIVE_FAILURES) {
          dead = true;
          cancel?.();
          setState(current => ({ ...current, unavailable: true }));
        }
      }
    }, intervalMs, scheduler.schedule, scheduler.cancel);

    return () => {
      dead = true;
      cancel?.();
    };
  }, [sessionId, enabled, intervalMs, fetchers, scheduler]);

  return state;
}
