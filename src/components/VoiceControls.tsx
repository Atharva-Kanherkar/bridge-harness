import { useEffect, useRef } from "react";
import { Mic, Square, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { VoiceState, VoiceView } from "@/voiceDictation";
import type { VoiceRecoveryAction } from "@/protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";
import { useLatest } from "../useLatest";

export function voiceIsBusy(state: VoiceState): boolean {
  return state === "starting" || state === "recording" || state === "stopping";
}

/** The same controls serve the composer and the settings microphone test. */
export function VoiceControls({ state, available, disabled, unavailableReason, providerLabel, error, recovery,
  onStart, onStop, onCancel, onRetry, onSetup }: {
  state: VoiceState;
  available: boolean;
  disabled?: boolean;
  unavailableReason?: string;
  providerLabel?: string;
  error?: string;
  recovery?: VoiceRecoveryAction;
  onStart(): void;
  onStop(): void;
  onCancel?(): void;
  onRetry?(): void;
  onSetup?(): void;
}) {
  const busy = voiceIsBusy(state);
  const pointer = useRef<{ id: number; at: number; alreadyActive: boolean }>();
  const heldKey = useRef(false);
  const latest = useLatest({ onStop, onCancel });
  useEffect(() => {
    if (!busy) { heldKey.current = false; pointer.current = undefined; }
  }, [busy]);
  useEffect(() => {
    // A key released after focus moved must not leave the microphone running.
    const release = (event: KeyboardEvent) => {
      if (event.key === " " && heldKey.current) { heldKey.current = false; latest.current.onStop(); }
    };
    const blur = () => {
      if (heldKey.current || pointer.current) {
        heldKey.current = false;
        pointer.current = undefined;
        latest.current.onCancel?.();
      }
    };
    window.addEventListener("keyup", release);
    window.addEventListener("blur", blur);
    return () => { window.removeEventListener("keyup", release); window.removeEventListener("blur", blur); };
  }, [latest]);
  const recoveryAction = error && recovery !== "setup" ? "retry" : recovery;
  const recover = recoveryAction === "setup" && onSetup ? { label: "Set up voice", run: onSetup }
    : recoveryAction === "retry" && onRetry ? { label: "Retry voice", run: onRetry } : undefined;
  const pending = state === "starting" || state === "stopping";

  return <>
    {!busy && recover && <Button variant="quiet" size="xs" tooltip={unavailableReason} onClick={recover.run} disabled={disabled}
     >{recover.label}</Button>}
    <Button variant="quiet" size="icon-sm" data-voice-control disabled={disabled || (!available && !busy) || state === "stopping"}
      aria-label={busy ? "Stop dictating" : "Hold to dictate"} aria-pressed={busy} loading={pending}
      aria-description={!available && !busy ? unavailableReason : providerLabel}
      tooltip={!available && !busy ? unavailableReason ?? "Dictation unavailable" : [providerLabel, busy ? "Finish dictation" : "Click to dictate, or hold to talk"].filter(Boolean).join(". ")}
      onPointerDown={event => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        pointer.current = { id: event.pointerId, at: Date.now(), alreadyActive: busy };
        if (!busy) onStart();
      }}
      onPointerUp={event => {
        const press = pointer.current;
        if (!press || press.id !== event.pointerId) return;
        pointer.current = undefined;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        if (press.alreadyActive || Date.now() - press.at >= 300) onStop();
      }}
      onPointerCancel={() => { pointer.current = undefined; onCancel?.(); }}
      onLostPointerCapture={() => { if (pointer.current) { pointer.current = undefined; onCancel?.(); } }}
      onClick={event => { if (event.detail === 0) { if (busy) onStop(); else onStart(); } }}
      onKeyDown={event => {
        if (event.key !== " ") return;
        event.preventDefault();
        if (!event.repeat) { heldKey.current = true; if (!busy) onStart(); }
      }}
      onKeyUp={event => {
        if (event.key === " ") {
          event.preventDefault();
          if (heldKey.current) { heldKey.current = false; onStop(); }
        }
      }}
    >{state === "recording" ? <Square className="h-3 w-3 fill-current" aria-hidden="true" /> : <Mic className="h-4 w-4" aria-hidden="true" />}</Button>
    {busy && onCancel && <Button variant="quiet" size="icon-sm" tooltip="Cancel dictation (Esc)" data-voice-control onClick={onCancel} aria-label="Cancel dictation"
     ><X className="h-4 w-4" aria-hidden="true" /></Button>}
  </>;
}

export function VoiceStatus({ state, startingPhase, audioLevel = 0, error, notice }: Pick<VoiceView, "state" | "startingPhase" | "audioLevel" | "error" | "notice">) {
  const busy = voiceIsBusy(state);
  return <>
    {busy && <div className="flex min-h-6 flex-wrap items-center justify-between gap-x-3 gap-y-1 px-1 text-xs text-muted-foreground">
      <span className="inline-flex items-center gap-2">
        <span aria-hidden="true" className="flex h-4 w-5 items-center justify-between gap-0.5">
          {["h-1.5", "h-3", "h-4", "h-3", "h-1.5"].map((height, index) => <span key={index}
            className={cn("w-0.5 rounded-full transition-opacity motion-reduce:transition-none", height,
              state !== "stopping" && audioLevel > index / 5 ? "bg-foreground opacity-100" : "bg-muted-foreground opacity-25")} />)}
        </span>
        {state === "starting" && startingPhase !== "microphone" || state === "stopping"
          ? <SkeletonRegion label={state === "stopping" ? "Finishing" : "Loading speech model"}><Skeleton className="h-2.5 w-24" /></SkeletonRegion>
          : <span role="status">{state === "starting" ? "Allow microphone access" : "Listening"}</span>}
      </span>
      <span className="text-caption">{state === "stopping" ? "Esc to cancel" : "Enter to finish · Esc to cancel"}</span>
    </div>}
    {error && <p role="alert" className="px-1 py-1 text-xs leading-relaxed text-destructive">{error}</p>}
    {notice && !error && <p role="status" className="px-1 py-1 text-xs leading-relaxed text-muted-foreground">{notice}</p>}
  </>;
}
