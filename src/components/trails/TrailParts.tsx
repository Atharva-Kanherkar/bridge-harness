import type { ReactNode } from "react";
import type { TrailState } from "@/protocol/generated/protocol";
import { TRAIL_STAGES, TRAIL_STATE_LABEL, stageLabel } from "@/trails";
import { clsx as cn } from "clsx";

/** Four segments: Plan, Chats, Pull request, Shipped. The reached ones fill;
 * when the trail needs the person, the current segment turns amber. */
export function StageBar({ stage, state, className }: { stage: number; state: TrailState; className?: string }) {
  return (
    <span className={cn("flex items-center gap-1", className)} role="img" aria-label={`Stage: ${stageLabel(stage)}`}>
      {TRAIL_STAGES.map((name, index) => {
        const reached = index < stage;
        const current = index === stage - 1;
        return (
          <span
            key={name}
            className={cn(
              "h-1.5 w-5 rounded-full",
              current && state === "needsYou" ? "bg-warning" : state === "shipped" && reached ? "bg-success" : reached ? "bg-foreground/60" : "bg-muted",
            )}
          />
        );
      })}
    </span>
  );
}

const STATE_INK: Record<TrailState, string> = {
  needsYou: "text-warning",
  inProgress: "text-muted-foreground",
  shipped: "text-success",
};

export function StateWord({ state, className }: { state: TrailState; className?: string }) {
  return <span className={cn("text-caption font-medium", STATE_INK[state], className)}>{TRAIL_STATE_LABEL[state]}</span>;
}

export function Chip({ children, tone = "plain", title, className }: { children: ReactNode; tone?: "plain" | "warning" | "danger" | "success"; title?: string; className?: string }) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-6 max-w-full items-center gap-1.5 truncate rounded-md bg-muted px-2 text-caption font-medium",
        tone === "warning" ? "text-warning" : tone === "danger" ? "text-destructive" : tone === "success" ? "text-success" : "text-muted-foreground",
        className,
      )}
    >
      {children}
    </span>
  );
}

export function RailSection({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-xl bg-card p-4">
      <div className="mb-2.5 flex items-baseline justify-between gap-2">
        <h3 className="text-caption font-medium uppercase tracking-wide text-faint">{title}</h3>
        {aside && <span className="shrink-0 font-mono text-caption text-faint">{aside}</span>}
      </div>
      {children}
    </section>
  );
}
