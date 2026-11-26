import { useEffect, useMemo, useRef, useState } from "react";
import { Ban, ChevronRight, CircleAlert, FileSearch, SquareTerminal } from "lucide-react";
import { SkeletonDot, SkeletonRegion, SkeletonText } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../../api";
import { friendlyError } from "../../errors";
import { GithubMarkdown } from "../github/GithubMarkdown";
import { harnessLabel } from "../github/reviewLabels";
import type { ReviewActivityEntry, ReviewPass } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { Tip } from "@/components/ui/tooltip";

// What each reviewer pass did, as a chat: Bridge's request, then the
// reviewer's messages, tools and commands in order. Live while the review
// runs (`reviews/activity`), and kept afterwards.

const key = (entry: ReviewActivityEntry) => `${entry.passId}:${entry.seq}`;

/** Merge entries by pass and sequence, keeping each once. */
export function mergeActivity(current: ReviewActivityEntry[], incoming: ReviewActivityEntry[]): ReviewActivityEntry[] {
  const seen = new Map(current.map(entry => [key(entry), entry]));
  for (const entry of incoming) seen.set(key(entry), entry);
  return [...seen.values()];
}

function PromptBubble({ entry }: { entry: ReviewActivityEntry }) {
  const [open, setOpen] = useState(false);
  return <li className="flex justify-end">
    <div className="max-w-[85%] rounded-xl rounded-tr-sm border border-border bg-muted/50 px-3 py-2 text-meta">
      <Button variant="link" size="inline" aria-expanded={open} onClick={() => setOpen(value => !value)}>
        <ChevronRight size={12} aria-hidden="true" className={cn("transition-transform", open && "rotate-90")} />
        {entry.title ?? "Bridge's request"}
      </Button>
      {open && <pre className="mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap font-mono text-caption text-muted-foreground">{entry.text}</pre>}
    </div>
  </li>;
}

function Step({ entry }: { entry: ReviewActivityEntry }) {
  const Icon = entry.kind === "command" ? SquareTerminal : entry.kind === "denied" ? Ban : entry.kind === "error" ? CircleAlert : FileSearch;
  // An error step carries the provider's raw failure; a command its own text.
  const text = entry.kind === "error" && entry.text ? friendlyError(entry.text, "The reviewer hit an error.") : entry.text;
  return <li className={cn("flex min-w-0 items-baseline gap-2 px-1 text-caption",
    entry.kind === "error" ? "text-destructive" : entry.kind === "denied" ? "text-warning" : "text-muted-foreground")}>
    <Icon size={12} aria-hidden="true" className="shrink-0 translate-y-0.5" />
    <span className={cn("shrink-0 font-medium", entry.kind === "command" && "font-mono")}>{entry.kind === "error" ? "Error" : entry.title}</span>
    {text && <Tip label={text}><span className={cn("min-w-0 truncate", entry.kind !== "error" && "font-mono")}>{text}</span></Tip>}
  </li>;
}

function Message({ entry, repositoryUrl }: { entry: ReviewActivityEntry; repositoryUrl?: string }) {
  return <li className="max-w-[92%] rounded-xl rounded-tl-sm bg-card px-3 py-2 text-meta text-foreground shadow-xs ring-1 ring-border">
    {entry.title && <p className="mb-1 text-caption font-medium text-muted-foreground">{entry.title}</p>}
    <GithubMarkdown text={entry.text ?? ""} repositoryUrl={repositoryUrl} />
  </li>;
}

export function ReviewActivity({ jobId, passes, running, repositoryUrl }: {
  jobId: string;
  passes: ReviewPass[];
  /** The job is still working: show that, and keep listening. */
  running: boolean;
  repositoryUrl?: string;
}) {
  const [entries, setEntries] = useState<ReviewActivityEntry[]>();
  const [error, setError] = useState<string>();
  const bottom = useRef<HTMLDivElement>(null);
  const following = useRef(true);

  // Subscribe first, then read, so nothing between the two is lost.
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    setEntries(undefined); setError(undefined);
    void (async () => {
      try {
        const stop = await bridgeApi.onReviewActivity(({ jobId: id, entry }) => {
          if (active && id === jobId) setEntries(current => mergeActivity(current ?? [], [entry]));
        });
        if (!active) { stop(); return; }
        unlisten = stop;
        const result = await bridgeApi.reviewActivity(jobId);
        if (active) setEntries(current => mergeActivity(result.entries, current ?? []));
      } catch (cause) {
        if (active) setError(friendlyError(cause, "Couldn't load the review's activity."));
      }
    })();
    return () => { active = false; unlisten?.(); };
  }, [jobId]);

  // Stay at the newest entry unless the person scrolled up to read.
  useEffect(() => {
    if (following.current) bottom.current?.scrollIntoView?.({ block: "end" });
  }, [entries?.length]);

  const threads = useMemo(() => {
    const byPass = new Map<string, ReviewActivityEntry[]>();
    for (const entry of entries ?? []) byPass.set(entry.passId, [...(byPass.get(entry.passId) ?? []), entry]);
    // Passes in their own order; a pass the detail has not loaded yet last.
    const order = passes.map(pass => pass.id);
    return [...byPass.entries()]
      .sort(([a], [b]) => (order.indexOf(a) === -1 ? Infinity : order.indexOf(a)) - (order.indexOf(b) === -1 ? Infinity : order.indexOf(b)))
      .map(([passId, items]) => ({ pass: passes.find(pass => pass.id === passId), items: items.sort((a, b) => a.seq - b.seq) }));
  }, [entries, passes]);

  if (error) return <p role="alert" className="text-meta text-destructive">{error}</p>;
  if (!entries) return <SkeletonRegion label="Loading activity" className="space-y-3"><SkeletonText lines={4} /></SkeletonRegion>;
  return <div data-review-activity={jobId} className="grid gap-4" onScroll={event => {
    const target = event.currentTarget;
    following.current = target.scrollHeight - target.scrollTop - target.clientHeight < 48;
  }}>
    {threads.length === 0 && !running && <p className="text-meta text-muted-foreground">No activity was recorded for this review.</p>}
    {threads.length === 0 && running && <p className="text-meta text-muted-foreground">Nothing yet: the reviewer's messages and tool calls appear here as it works.</p>}
    {threads.map(({ pass, items }) => <section key={items[0].passId} aria-label={`${pass ? harnessLabel(pass.harness) : "Reviewer"} activity`} className="grid gap-2">
      <h4 className="flex items-center gap-2 text-caption font-medium text-muted-foreground">
        <span className="rounded-sm border border-border bg-background px-1.5 py-px text-foreground">{pass ? harnessLabel(pass.harness) : "Reviewer"}</span>
        {pass && pass.attempt > 1 && <span>attempt {pass.attempt}</span>}
        {pass && (pass.model || pass.effort) && <span className="font-mono">{pass.model ?? "default model"} · {pass.effort ?? "default effort"}</span>}
        <span className="tabular-nums">{items.filter(item => item.kind === "tool" || item.kind === "command").length} steps</span>
      </h4>
      <ul className="grid gap-1.5">
        {items.map(entry => entry.kind === "prompt"
          ? <PromptBubble key={key(entry)} entry={entry} />
          : entry.kind === "message" ? <Message key={key(entry)} entry={entry} repositoryUrl={repositoryUrl} /> : <Step key={key(entry)} entry={entry} />)}
      </ul>
    </section>)}
    {running && <p role="status" className="flex items-center gap-2 text-meta text-muted-foreground">
      <SkeletonDot /> Working…
    </p>}
    <div ref={bottom} />
  </div>;
}
