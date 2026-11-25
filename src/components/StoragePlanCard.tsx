// The storage agent's one way to change the disk: a ```storage-plan block it
// writes into a reply, drawn here as a card the person approves. Paths go
// through Bridge's own Trash mover (which refuses system folders, keychains,
// and Bridge's data); commands go back to the agent as an explicit approval.
//
// Inside the Storage page the card can act. Anywhere else (the chat opened as
// a full conversation) it is a read-only record of what was proposed.

import { createContext, useContext, useMemo, useState } from "react";
import { Check, FolderOpen, MousePointerClick, Terminal, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { diskBytes, displayPath, parseStoragePlan, type StoragePlanCommand, type StoragePlanItem } from "../diskSpace";
import { StatusPill } from "./settings/kit";
import { Tip } from "@/components/ui/tooltip";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";

/** What the Storage page lends a plan card. */
export interface StorageAgentHost {
  home: string | undefined;
  busy: boolean;
  /** Paths already moved to the Trash this visit, so a re-drawn card stays honest. */
  trashed: ReadonlySet<string>;
  /** Move to the Trash; resolves with a one-line outcome. */
  trash: (items: StoragePlanItem[]) => Promise<string>;
  /** Add to the page's selection. */
  select: (items: StoragePlanItem[]) => void;
  /** Show an item's folder in the explorer. */
  reveal: (path: string) => void;
  /** Send a message to the storage agent. */
  reply: (text: string) => void;
}

export const StorageAgentContext = createContext<StorageAgentHost | null>(null);


export function StoragePlanCard({ body }: { body: string }) {
  const host = useContext(StorageAgentContext);
  const plan = useMemo(() => parseStoragePlan(body, host?.home), [body, host?.home]);
  // Only what rebuilds itself starts ticked; the person opts in to the rest.
  // Kept as overrides because the plan can parse later than the first render
  // (still streaming, or the page has not learned the home folder yet).
  const [overrides, setOverrides] = useState<ReadonlyMap<string, boolean>>(new Map());
  const [approved, setApproved] = useState<Set<string>>(new Set());
  const [outcome, setOutcome] = useState<string | null>(null);
  const [working, setWorking] = useState(false);

  if (!plan) {
    return <div className="my-3 rounded-xl border border-dashed border-border px-4 py-3 text-caption text-muted-foreground motion-safe:animate-pulse">Drafting a cleanup plan…</div>;
  }
  const trashed = host?.trashed ?? new Set<string>();
  const live = plan.items.filter(item => !trashed.has(item.path));
  const isTicked = (item: StoragePlanItem) => overrides.get(item.path) ?? item.safety === "safe";
  const chosen = live.filter(isTicked);
  const chosenBytes = chosen.reduce((total, item) => total + (item.sizeBytes ?? 0), 0);
  const planBytes = plan.items.reduce((total, item) => total + (item.sizeBytes ?? 0), 0) + plan.commands.reduce((total, command) => total + (command.frees ?? 0), 0);
  const toggle = (item: StoragePlanItem) => setOverrides(current => new Map(current).set(item.path, !isTicked(item)));
  const trash = async () => {
    if (!host || chosen.length === 0) return;
    setWorking(true);
    try { setOutcome(await host.trash(chosen)); } finally { setWorking(false); }
  };
  const approve = (commands: StoragePlanCommand[]) => {
    if (!host || commands.length === 0) return;
    setApproved(current => new Set([...current, ...commands.map(command => command.run)]));
    host.reply(commands.length === 1
      ? `Approved: run \`${commands[0].run}\``
      : `Approved: run these in order\n${commands.map(command => `- \`${command.run}\``).join("\n")}`);
  };
  const pendingCommands = plan.commands.filter(command => !approved.has(command.run));

  return <section aria-label={`Plan: ${plan.title}`} className="my-3 overflow-hidden rounded-xl border border-border bg-card text-ui">
    <header className="flex items-baseline gap-3 border-b border-border px-4 py-3">
      <h4 className="min-w-0 flex-1 truncate font-medium text-foreground">{plan.title}</h4>
      {planBytes > 0 && <span className="shrink-0 font-display text-lg font-semibold tabular-nums text-foreground">{diskBytes(planBytes)}</span>}
    </header>

    {plan.items.length > 0 && <ul className="divide-y divide-border/60">
      {plan.items.map(item => {
        const gone = trashed.has(item.path);
        const ticked = !gone && isTicked(item);
        const name = item.path.split("/").filter(Boolean).at(-1) ?? item.path;
        return <li key={item.path} className={cn("flex items-start gap-3 px-4 py-2.5", gone && "opacity-50")}>
          {host
            ? <Checkbox
              className="mt-0.5"
              aria-label={`Include ${name}`}
              checked={ticked}
              disabled={gone}
              onCheckedChange={() => toggle(item)}
            />
            : <span aria-hidden="true" className={cn("mt-1.5 size-2 shrink-0 rounded-full", item.safety === "safe" ? "bg-success" : "bg-warning")} />}
          <div className="min-w-0 flex-1">
            <p className="flex items-baseline gap-2">
              <span className="truncate text-foreground">{name}</span>
              {gone
                ? <StatusPill>In Trash</StatusPill>
                : <StatusPill tone={item.safety === "safe" ? "success" : "warning"}>{item.safety === "safe" ? "Rebuilds" : "Review"}</StatusPill>}
            </p>
            {item.why && <p className="mt-0.5 text-caption text-muted-foreground">{item.why}</p>}
            <Tip label={item.path}><p className="mt-0.5 truncate font-mono text-caption text-faint">{displayPath(item.path, host?.home)}</p></Tip>
          </div>
          <div className="flex shrink-0 flex-col items-end gap-1">
            <span className="tabular-nums text-foreground">{diskBytes(item.sizeBytes)}</span>
            {host && <Button variant="quiet" size="icon-2xs" onClick={() => host.reveal(item.path)} aria-label={`Show ${name} on the page`}><FolderOpen size={13} aria-hidden="true" /></Button>}
          </div>
        </li>;
      })}
    </ul>}

    {plan.commands.length > 0 && <ul className={cn("divide-y divide-border/60", plan.items.length > 0 && "border-t border-border")}>
      {plan.commands.map(command => {
        const done = approved.has(command.run);
        return <li key={command.run} className="flex items-start gap-3 px-4 py-2.5">
          <Terminal size={14} className="mt-0.5 shrink-0 text-info" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <Tip label={command.run}><code className="block truncate font-mono text-meta text-foreground">{command.run}</code></Tip>
            {command.why && <p className="mt-0.5 text-caption text-muted-foreground">{command.why}</p>}
          </div>
          <div className="flex shrink-0 flex-col items-end gap-1">
            {command.frees !== null && <span className="tabular-nums text-foreground">{diskBytes(command.frees)}</span>}
            {host && (done
              ? <span className="inline-flex items-center gap-1 text-caption text-success"><Check size={12} aria-hidden="true" />Approved</span>
              : <Button variant="quiet" size="2xs" onClick={() => approve([command])}>Run it</Button>)}
          </div>
        </li>;
      })}
    </ul>}

    {host
      ? <footer className="flex flex-wrap items-center gap-2 border-t border-border bg-muted/40 px-3 py-2">
        {live.length > 0 && <>
          <Button variant="default" size="xs" disabled={working || host.busy || chosen.length === 0} onClick={() => void trash()}>
            <Trash2 size={13} aria-hidden="true" />
            {chosen.length === 0 ? "Nothing ticked" : `Move ${chosen.length} to Trash · ${diskBytes(chosenBytes)}`}
          </Button>
          <Button variant="quiet" size="xs" disabled={chosen.length === 0} onClick={() => host.select(chosen)}>
            <MousePointerClick size={13} aria-hidden="true" />Select on page
          </Button>
        </>}
        {pendingCommands.length > 1 && <Button variant="quiet" size="xs" onClick={() => approve(pendingCommands)}>Run all {pendingCommands.length}</Button>}
        {outcome && <p role="status" className="w-full px-1 pt-1 text-caption text-muted-foreground">{outcome}</p>}
      </footer>
      : <p className="border-t border-border px-4 py-2 text-caption text-muted-foreground">Open Settings → Data &amp; storage to act on this plan.</p>}
  </section>;
}
