import clsx from "clsx";
import { Kbd } from "@/components/ui/kbd";
import { chordLabel } from "../keymap";

/** What Enter does right now: send a new turn, or, mid-turn, steer or queue. */
export type ComposerKeyState = "idle" | "steer" | "queue";

export type ComposerKeyHint = { keys: string; label: string };

/**
 * The keys the composer answers to in this state. Idle, Enter sends and
 * Shift+Enter breaks the line; while a turn runs, Enter steers or queues by the
 * chat's "While running" setting, and the interrupt chord stops the turn. The
 * chord is read from the keymap, so the hint cannot drift from the binding.
 */
export function composerKeyHints(state: ComposerKeyState): ComposerKeyHint[] {
  if (state === "idle") return [{ keys: "↵", label: "Send" }, { keys: "⇧↵", label: "New line" }];
  return [{ keys: "↵", label: state === "steer" ? "Steer" : "Queue" }, { keys: chordLabel("interrupt-turn"), label: "Stop" }];
}

export function ComposerKeyHints({ state, className }: { state: ComposerKeyState; className?: string }) {
  return (
    <span data-composer-keys={state} className={clsx("inline-flex items-center gap-3 text-caption text-faint", className)}>
      {composerKeyHints(state).map(hint => (
        <span key={hint.label} className="inline-flex items-center gap-1">
          <Kbd>{hint.keys}</Kbd>
          {hint.label}
        </span>
      ))}
    </span>
  );
}
