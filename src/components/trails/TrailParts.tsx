import { useCallback, useEffect, useRef, useState, type ComponentType, type CSSProperties, type ReactNode } from "react";
import { CircleDot, FileText, GitPullRequest, ListChecks, MessagesSquare, Send, Trash2 } from "lucide-react";
import type { TrailEventKind, TrailState } from "@/protocol/generated/protocol";
import { TRAIL_STAGES, TRAIL_STATE_LABEL, stageLabel } from "@/trails";
import { clsx as cn } from "clsx";

export const KIND_LABEL: Record<TrailEventKind, string> = {
  issue: "Issue",
  note: "Note",
  chat: "Chat",
  pullRequest: "Pull request",
  checks: "Checks",
  message: "Message",
  removed: "Removed",
};

export const KIND_ICON: Record<TrailEventKind, ComponentType<{ className?: string }>> = {
  issue: CircleDot,
  note: FileText,
  chat: MessagesSquare,
  pullRequest: GitPullRequest,
  checks: ListChecks,
  message: Send,
  removed: Trash2,
};

/** Ink for an event's tone; `plain` stays quiet. */
export const TONE_INK = { plain: "text-muted-foreground", success: "text-success", warning: "text-warning", danger: "text-destructive" } as const;

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
        <h3 className="text-caption font-medium text-faint">{title}</h3>
        {aside && <span className="shrink-0 text-caption tabular-nums text-faint">{aside}</span>}
      </div>
      {children}
    </section>
  );
}

/** How far the page scrolls before the condensed bar slides in: about where
 * the page's own title has gone under the top edge. */
export const TRAIL_BAR_THRESHOLD = 72;

/** The entrance delay for the nth row of a list. It stops growing after a
 * dozen rows, so a long timeline never waits on its tail. */
export function riseDelay(index: number): CSSProperties {
  return { "--trail-rise-delay": `${Math.min(index, 12) * 32}ms` } as CSSProperties;
}

function prefersReducedMotion(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The Trails screens' scroller. The app shell's `main` clips, so each
 * destination brings its own. Once the page's title scrolls away, a blurred
 * bar slides in with the title and the page's main action, and a hairline
 * along its foot tracks how far down the page is. Scroll progress is a CSS
 * variable written on the scroller, so scrolling does not render React; only
 * crossing the threshold does. `resetKey` sends the scroll back to the top
 * when the page starts showing something else. */
export function TrailScroll({ title, action, wide = false, resetKey, children }: {
  title: ReactNode;
  action?: ReactNode;
  /** Match the page's width: the trail page is wider than the list. */
  wide?: boolean;
  resetKey?: string | null;
  children: ReactNode;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState(false);
  // What the bar last showed: a scroll that does not cross the threshold
  // must not reach React at all, not even for a bail-out render.
  const shownRef = useRef(false);

  const measure = useCallback(() => {
    const node = scrollRef.current;
    if (!node) return;
    const range = node.scrollHeight - node.clientHeight;
    const progress = range > 0 ? Math.min(1, Math.max(0, node.scrollTop / range)) : 0;
    node.style.setProperty("--trail-scroll-progress", progress.toFixed(4));
    const next = node.scrollTop > TRAIL_BAR_THRESHOLD;
    if (next !== shownRef.current) { shownRef.current = next; setShown(next); }
  }, []);

  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    node.addEventListener("scroll", measure, { passive: true });
    measure();
    return () => node.removeEventListener("scroll", measure);
  }, [measure]);

  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    node.scrollTop = 0;
    measure();
  }, [resetKey, measure]);

  // React 18 has no `inert` prop: set the attribute, so a hidden bar's
  // buttons are out of the tab order as well as out of reach.
  useEffect(() => { barRef.current?.toggleAttribute("inert", !shown); }, [shown]);

  function toTop() {
    const node = scrollRef.current;
    if (!node) return;
    const behavior = prefersReducedMotion() ? "auto" : "smooth";
    if (typeof node.scrollTo === "function") node.scrollTo({ top: 0, behavior });
    else node.scrollTop = 0;
  }

  return (
    <div ref={scrollRef} data-trail-scroll="" className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain">
      <div className="sticky top-0 z-20 h-0">
        <div
          ref={barRef}
          data-trail-bar=""
          data-shown={shown ? "true" : "false"}
          aria-hidden={shown ? undefined : true}
          className={cn(
            "pointer-events-none absolute inset-x-0 top-0 -translate-y-2 border-b border-border/70 bg-background/80 opacity-0 backdrop-blur-xl backdrop-saturate-150",
            "transition-[opacity,translate] duration-200 ease-out-bridge",
            "data-[shown=true]:pointer-events-auto data-[shown=true]:translate-y-0 data-[shown=true]:opacity-100",
          )}
        >
          <div className={cn("mx-auto flex h-11 w-full items-center gap-3 px-5 sm:px-8", wide ? "max-w-page-wide" : "max-w-page")}>
            <button
              type="button"
              onClick={toTop}
              title="Back to top"
              className="min-w-0 flex-1 truncate rounded-sm text-left text-ui font-semibold tracking-tight text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {title}
            </button>
            {action && <div className="flex shrink-0 items-center gap-1">{action}</div>}
          </div>
          <span
            aria-hidden="true"
            data-trail-progress=""
            className="absolute inset-x-0 -bottom-px h-px origin-left scale-x-[var(--trail-scroll-progress,0)] bg-foreground/45 motion-reduce:hidden"
          />
        </div>
      </div>
      {children}
    </div>
  );
}
