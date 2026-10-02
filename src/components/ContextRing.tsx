// The chat's context ring: how full the selected session's live window is,
// beside the send button where the usage dot used to sit. Quota lives in the
// sidebar rail now; this ring is only ever about the model's window.
import { cn } from "@/lib/utils";
import { contextTone, TONE_TEXT, type ContextTone } from "../contextWindows";

export function ContextGauge({ percent, size = 20, className, fillClassName }: {
  percent: number | null | undefined;
  size?: number;
  className?: string;
  /** Ink for the filled arc; defaults to the pressure tone the composer uses. */
  fillClassName?: string;
}) {
  const radius = 8;
  const circumference = 2 * Math.PI * radius;
  const drawn = percent == null ? 0 : Math.max(0, Math.min(100, percent));
  const tone: ContextTone = contextTone(percent);
  return <svg width={size} height={size} viewBox="0 0 20 20" className={cn("shrink-0 -rotate-90", className)} aria-hidden="true">
    <circle cx="10" cy="10" r={radius} fill="none" strokeWidth="2.25" stroke="currentColor" className="text-foreground/10" />
    {percent != null && <circle
      cx="10" cy="10" r={radius} fill="none" strokeWidth="2.25" strokeLinecap="round" stroke="currentColor"
      strokeDasharray={circumference} strokeDashoffset={circumference * (1 - drawn / 100)}
      className={cn(fillClassName ?? TONE_TEXT[tone], "transition-[stroke-dashoffset,color] duration-700 ease-out motion-reduce:transition-none")}
    />}
  </svg>;
}

export function ContextRing({ percent, model, active = false, onOpen }: { percent: number | null | undefined; model?: string | null; active?: boolean; onOpen: () => void }) {
  const known = percent != null;
  const tone = contextTone(percent);
  const label = known ? `Context window: ${Math.round(percent)}% used` : "Context window: no reading yet";
  return <button
    type="button"
    onClick={onOpen}
    aria-label={label}
    aria-pressed={active}
    title={known ? `${Math.round(percent)}% of ${model ?? "this model"}'s context window` : "No context reading yet"}
    className={cn(
      "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full px-1.5 transition-colors duration-150 hover:bg-accent active:scale-95",
      active && "bg-accent",
    )}
  >
    <ContextGauge percent={percent} />
    <span className={cn("min-w-[2.25rem] pr-0.5 text-left font-mono text-[11px] tabular-nums", known ? TONE_TEXT[tone] : "text-muted-foreground")}>
      {known ? `${Math.round(percent)}%` : "–"}
    </span>
  </button>;
}
