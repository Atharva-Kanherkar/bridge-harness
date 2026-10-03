import type { SessionStartupPhase } from "./types";

/**
 * Pure narration state for a cold-start status row. Kept free of timers and
 * subscriptions so the label sequence, elapsed-counter visibility, and the
 * collapse-after-first-token handoff are testable against synthetic
 * timestamps rather than real clocks.
 */

const COLLAPSE_LINGER_MS = 2000;
// A short wait is just thinking; a counter only earns its place once the wait
// is long enough that the reader starts to wonder.
const ELAPSED_VISIBLE_AFTER_MS = 10_000;

export interface NarrationInput {
  /** A message is pending and nothing has visibly started yet. */
  hasPendingWork: boolean;
  /** Some item in the transcript is now streaming or in progress. */
  streaming: boolean;
  /** A model switch is in flight: the incoming model's display label. Wins
   *  over every other label, and mounts the row even with no pending work —
   *  the switch is Bridge's own activity, not the agent's. */
  switchingToLabel: string | null;
  /** The harness is compacting its context: when that began, or `null`. Wins
   *  over every phase label and over the first-token handoff, because during a
   *  compaction nothing is being thought or streamed, and "Thinking" would be
   *  the one word that is wrong. A switch still outranks it. */
  compactingSince: number | null;
  /** The furthest cold-start phase observed so far, or `null` if none has. */
  latestPhase: SessionStartupPhase | null;
  /** When `hasPendingWork` first became true, or `null` before that. */
  startedAt: number | null;
  /** When `streaming` first became true, or `null` before that. */
  streamStartedAt: number | null;
  now: number;
  reducedMotion: boolean;
}

export interface NarrationView {
  /** Whether the status row should be in the tree at all. */
  mounted: boolean;
  /** True once streaming has begun: the label disappears and only the
   *  animation node (or its reduced-motion stand-in) remains, for
   *  `COLLAPSE_LINGER_MS` before `mounted` goes false. */
  collapsed: boolean;
  label: string;
  showElapsed: boolean;
  elapsedSeconds: number;
  reducedMotion: boolean;
}

const MOUNTED_NONE: NarrationView = {
  mounted: false,
  collapsed: false,
  label: "",
  showElapsed: false,
  elapsedSeconds: 0,
  reducedMotion: false,
};

/// No harness or model name in any label: the row's mark already says who.
function phaseLabel(phase: SessionStartupPhase | null): string {
  switch (phase) {
    case "spawning": return "Starting…";
    case "handshake": return "Connecting…";
    case "session_open": return "Opening session…";
    default: return "Thinking";
  }
}

export function computeNarration(input: NarrationInput): NarrationView {
  const { hasPendingWork, streaming, switchingToLabel, compactingSince, latestPhase, startedAt, streamStartedAt, now, reducedMotion } = input;
  // A switch in flight owns the row outright: it mounts without pending work,
  // never collapses (there is no stream to hand off to), and outranks the
  // phase labels — whatever the old provider is doing behind the scenes, the
  // truth on screen is that Bridge is switching models. Otherwise the old
  // model kept claiming to read a message while it was being replaced.
  if (switchingToLabel) {
    const elapsedMs = startedAt !== null ? Math.max(0, now - startedAt) : 0;
    return {
      mounted: true,
      collapsed: false,
      label: `Switching to ${switchingToLabel}…`,
      showElapsed: elapsedMs >= ELAPSED_VISIBLE_AFTER_MS,
      elapsedSeconds: Math.floor(elapsedMs / 1000),
      reducedMotion,
    };
  }
  // Counted from the compaction's own start, not the turn's: an automatic one
  // begins mid-turn, and a counter reading the whole turn's age would claim a
  // compaction that just started has been running for minutes.
  if (compactingSince !== null) {
    const elapsedMs = Math.max(0, now - compactingSince);
    return {
      mounted: true,
      collapsed: false,
      label: "Compacting context…",
      showElapsed: elapsedMs >= ELAPSED_VISIBLE_AFTER_MS,
      elapsedSeconds: Math.floor(elapsedMs / 1000),
      reducedMotion,
    };
  }
  // Once streaming starts the row keeps the animation node mounted a beat
  // longer so the handoff to the transcript's own shimmer never reads as a
  // flash — but it never lingers past `hasPendingWork` going false.
  const lingering = streamStartedAt !== null && now - streamStartedAt < COLLAPSE_LINGER_MS;
  const mounted = hasPendingWork && (!streaming || lingering);
  if (!mounted) return { ...MOUNTED_NONE, reducedMotion };
  const collapsed = streamStartedAt !== null;
  const elapsedMs = startedAt !== null ? Math.max(0, now - startedAt) : 0;
  return {
    mounted: true,
    collapsed,
    label: collapsed ? "" : phaseLabel(latestPhase),
    showElapsed: !collapsed && elapsedMs >= ELAPSED_VISIBLE_AFTER_MS,
    elapsedSeconds: Math.floor(elapsedMs / 1000),
    reducedMotion,
  };
}
