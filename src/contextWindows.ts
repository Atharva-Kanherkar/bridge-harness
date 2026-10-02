import { useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import { startSerialPoll } from "./polling";
import { contextPressure, type ContextPressure } from "./usage";
import type { ContextWindowReading, ContextWindowSegment, ContextWindowsResult } from "./protocol/generated/protocol";

export type ContextTone = ContextPressure["level"];

/** The ring's tone, from the same thresholds the pressure card explains. */
export function contextTone(percent: number | null | undefined): ContextTone {
  return contextPressure(percent ?? undefined).level;
}

export const TONE_TEXT: Record<ContextTone, string> = {
  unknown: "text-muted-foreground",
  healthy: "text-foreground",
  elevated: "text-foreground",
  high: "text-warning",
  critical: "text-destructive",
};

export interface WindowComposition {
  /** Segments that occupy the window, largest first. */
  used: ContextWindowSegment[];
  /** Occupied tokens the harness did not attribute to any segment. */
  unattributed: number;
  /** Headroom: the window minus what is occupied. */
  free: number;
}

/** Split a reading into drawable parts without inventing any: segments only
 *  where the harness named them, the rest of the occupied space hatched. */
export function windowComposition(reading: ContextWindowReading): WindowComposition {
  const used = reading.segments.filter(segment => segment.kind === "used" && segment.tokens > 0);
  const attributed = used.reduce((sum, segment) => sum + segment.tokens, 0);
  const occupied = Math.max(0, Math.min(reading.usedTokens, reading.windowTokens));
  return {
    used,
    unattributed: Math.max(0, occupied - attributed),
    free: Math.max(0, reading.windowTokens - occupied),
  };
}

export function compactTokens(value: number): string {
  if (value >= 1_000_000) {
    const millions = value / 1_000_000;
    return `${Number.isInteger(millions) ? millions : millions.toFixed(1)}M`;
  }
  if (value >= 1_000) {
    const thousands = value / 1_000;
    return `${thousands >= 100 ? Math.round(thousands) : Math.round(thousands * 10) / 10}k`;
  }
  return String(Math.round(value));
}

const MISSING_METHOD = /not found|unknown command|no such command|unimplemented|method_not_found/i;
const MAX_FAILURES = 3;

type TimerHandle = ReturnType<typeof setTimeout>;

export interface ContextWindowsState {
  result: ContextWindowsResult | null;
  /** Polling gave up: the daemon lacks the method or kept failing. */
  unavailable: boolean;
}

/**
 * Serial polling for one chat's windows while the pane is visible, plus an
 * immediate refetch whenever `refreshKey` changes (the chat's context percent
 * moved). Switching chats discards the previous chat's result first.
 */
export function useContextWindows(
  sessionId: string | null | undefined,
  enabled: boolean,
  refreshKey: unknown,
  fetch: (sessionId: string) => Promise<ContextWindowsResult> = bridgeApi.contextWindows,
  intervalMs = 5_000,
  schedule: (callback: () => void, delay: number) => TimerHandle = setTimeout,
  cancel: (handle: TimerHandle) => void = clearTimeout,
): ContextWindowsState {
  const [state, setState] = useState<ContextWindowsState>({ result: null, unavailable: false });
  const failures = useRef(0);
  const shownFor = useRef<string | null>(null);

  useEffect(() => {
    if (shownFor.current !== sessionId) {
      shownFor.current = sessionId ?? null;
      failures.current = 0;
      setState({ result: null, unavailable: false });
    }
    if (!enabled || !sessionId) return;
    let dead = false;
    let stop: (() => void) | undefined;
    stop = startSerialPoll(async () => {
      try {
        const result = await fetch(sessionId);
        if (dead) return;
        failures.current = 0;
        setState({ result, unavailable: false });
      } catch (error) {
        if (dead) return;
        const message = error instanceof Error ? error.message : String(error);
        if (MISSING_METHOD.test(message) || ++failures.current >= MAX_FAILURES) {
          dead = true;
          stop?.();
          setState(current => ({ ...current, unavailable: true }));
        }
      }
    }, intervalMs, schedule, cancel);
    return () => {
      dead = true;
      stop?.();
    };
  }, [sessionId, enabled, refreshKey, fetch, intervalMs, schedule, cancel]);

  return state;
}
