import { useEffect, useState } from "react";
import { Check, ChevronDown, ChevronRight, Plus, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SegmentedGroup, SegmentedItem } from "@/components/ui/segmented";
import { SkeletonRows } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import type { MemoryRecord, TasteEvidence } from "../protocol/generated/protocol";

// The Memory app's Taste tab: rules learned from what you keep, edit and drop
// in reviews, and the ones you write yourself. Learned rules wait here for
// you; nothing is active until you approve it. Every rule opens onto the
// decisions behind it.

type Kind = "review_avoid" | "review_prefer" | "code_prefer";

export const KIND_LABEL: Record<Kind, string> = {
  review_avoid: "Review · don't raise",
  review_prefer: "Review · raise this way",
  code_prefer: "Code · write it this way",
};

const kindLabel = (kind: string) => KIND_LABEL[kind as Kind] ?? kind;

/** `project:owner/repo` as `owner/repo`; the account scope as every repository. */
export const repositoryLabel = (scopeKey: string) =>
  scopeKey.startsWith("project:") ? scopeKey.slice("project:".length) : scopeKey === "account:local" ? "Every repository" : scopeKey;

const VERDICT: Record<string, string> = {
  kept: "Kept", edited: "Edited", dropped: "Dropped", fixed: "Fixed", resolved: "Resolved", ignored: "Ignored", allowed: "Allowed", declined: "Declined",
};

function Evidence({ recordId, onError }: { recordId: string; onError: (message: string) => void }) {
  const [evidence, setEvidence] = useState<TasteEvidence>();
  useEffect(() => {
    let active = true;
    void bridgeApi.getTasteEvidence(recordId)
      .then(found => { if (active) setEvidence(found); })
      .catch(cause => { if (active) onError(friendlyError(cause, "Couldn't load the decisions behind this rule.")); });
    return () => { active = false; };
  }, [recordId, onError]);
  if (!evidence) return <SkeletonRows label="Loading the decisions" rows={2} framed={false} density="menu" />;
  if (!evidence.signals.length && !evidence.swept) return <p className="text-caption text-muted-foreground">You wrote this rule; no decisions are linked to it.</p>;
  return <ul aria-label="Decisions behind this rule" className="grid gap-1">
    {evidence.signals.map(signal => <li key={signal.id} className="flex flex-wrap items-baseline gap-x-2 text-caption">
      <span className="font-medium text-foreground">{VERDICT[signal.verdict] ?? signal.verdict}</span>
      <span className="min-w-0 flex-1 text-foreground/90">{signal.title}</span>
      {signal.path && <span className="font-mono text-muted-foreground">{signal.path}</span>}
      {signal.prNumber != null && <span className="text-muted-foreground">#{signal.prNumber}</span>}
    </li>)}
    {evidence.swept > 0 && <li className="text-caption text-faint">{evidence.swept} older {evidence.swept === 1 ? "decision" : "decisions"} no longer kept</li>}
  </ul>;
}

function RuleRow({ rule, busy, showRepository, onApprove, onReject, onDelete, onError }: {
  rule: MemoryRecord;
  busy: boolean;
  /** Name the repository on the row, where rules of several are mixed. */
  showRepository?: boolean;
  onApprove?: () => void;
  onReject?: () => void;
  onDelete?: () => void;
  onError: (message: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const decisions = rule.evidenceCount ?? 0;
  return <li data-taste-rule={rule.id} className="px-4 py-3">
    <div className="flex flex-wrap items-start gap-x-3 gap-y-1">
      <span className="mt-0.5 shrink-0 rounded-sm bg-muted px-1.5 text-caption font-medium text-muted-foreground">{kindLabel(rule.kind)}</span>
      <span className="min-w-0 flex-1 basis-64 text-ui text-foreground">{rule.body}</span>
      <span className="flex shrink-0 gap-1">
        {onApprove && <Button size="2xs" disabled={busy} onClick={onApprove}><Check className="size-3" aria-hidden="true" /> Approve</Button>}
        {onReject && <Button variant="ghost" size="2xs" disabled={busy} onClick={onReject}><X className="size-3" aria-hidden="true" /> Reject</Button>}
        {onDelete && <Button variant="ghost" size="2xs" disabled={busy} aria-label={`Delete rule: ${rule.body}`} onClick={onDelete}><Trash2 className="size-3" aria-hidden="true" /></Button>}
      </span>
    </div>
    <div className="mt-1 flex flex-wrap items-center gap-x-3 text-caption text-muted-foreground">
      {showRepository && <span className="font-mono">{repositoryLabel(rule.scopeKey)}</span>}
      {rule.pathGlobs?.length ? <span className="font-mono">{rule.pathGlobs.join(", ")}</span> : null}
      {rule.provenance === "user_explicit" ? <span>Written by you</span> : <span>Learned</span>}
      {rule.rationale && <span>{rule.rationale}</span>}
      {decisions > 0 && <Button variant="text" size="inline" aria-expanded={open} onClick={() => setOpen(current => !current)}>
        {open ? <ChevronDown className="size-3" aria-hidden="true" /> : <ChevronRight className="size-3" aria-hidden="true" />}
        From {decisions} {decisions === 1 ? "decision" : "decisions"}
      </Button>}
    </div>
    {open && <div className="mt-2 rounded-md border border-border bg-background px-3 py-2"><Evidence recordId={rule.id} onError={onError} /></div>}
  </li>;
}

function group(rules: MemoryRecord[]): [string, MemoryRecord[]][] {
  const byScope = new Map<string, MemoryRecord[]>();
  for (const rule of rules) byScope.set(rule.scopeKey, [...(byScope.get(rule.scopeKey) ?? []), rule]);
  return [...byScope.entries()].sort(([a], [b]) => (a === "account:local" ? 1 : b === "account:local" ? -1 : a.localeCompare(b)));
}

export function TasteRules({ onError, onCount }: { onError: (message: string) => void; onCount?: (waiting: number) => void }) {
  const [waiting, setWaiting] = useState<MemoryRecord[]>();
  const [active, setActive] = useState<MemoryRecord[]>();
  const [busy, setBusy] = useState(false);
  const [writing, setWriting] = useState(false);
  const [draft, setDraft] = useState({ repository: "", kind: "review_avoid" as Kind, body: "", paths: "" });

  const load = async () => {
    const [proposed, current] = await Promise.all([bridgeApi.listTasteRules("proposed"), bridgeApi.listTasteRules("active")]);
    setWaiting(proposed.rules);
    setActive(current.rules);
    onCount?.(proposed.rules.length);
  };
  useEffect(() => {
    // oxlint-disable-next-line react/set-state-in-effect -- load sets state only from its answers, after an await
    void load().catch(cause => onError(friendlyError(cause, "Couldn't load your taste rules.")));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const act = async (work: () => Promise<unknown>, failure: string) => {
    setBusy(true);
    try { await work(); await load(); } catch (cause) { onError(friendlyError(cause, failure)); } finally { setBusy(false); }
  };
  const review = (rule: MemoryRecord, approve: boolean) => act(
    () => bridgeApi.reviewMemoryRecords(approve ? [rule.id] : [], approve ? [] : [rule.id], rule.scopeKey),
    approve ? "Couldn't approve that rule." : "Couldn't reject that rule.",
  );
  const save = () => act(async () => {
    const repository = draft.repository.trim();
    await bridgeApi.addTasteRule({
      scopeKey: repository ? `project:${repository.replace(/^project:/, "")}` : "account:local",
      kind: draft.kind,
      body: draft.body.trim(),
      pathGlobs: draft.paths.split(",").map(path => path.trim()).filter(Boolean),
    });
    setDraft(current => ({ ...current, body: "", paths: "" }));
    setWriting(false);
  }, "Couldn't save that rule.");

  if (!waiting || !active) return <SkeletonRows label="Loading your taste" rows={3} />;
  return <div className="space-y-8">
    <p className="px-0.5 text-caption leading-relaxed text-muted-foreground">
      Rules learned from what you keep, edit and drop in reviews. The reviewer follows the review rules; coding agents follow the code rules. Nothing learned is active until you approve it.
    </p>
    {waiting.length > 0 && <section>
      <h2 className="mb-2 px-0.5 text-ui font-medium text-foreground">Waiting for you <span className="tabular-nums text-faint">{waiting.length}</span></h2>
      <ul aria-label="Taste rules waiting for you" className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
        {waiting.map(rule => <RuleRow key={rule.id} rule={rule} busy={busy} onError={onError} showRepository
          onApprove={() => void review(rule, true)} onReject={() => void review(rule, false)} />)}
      </ul>
    </section>}
    <section>
      <div className="mb-2 flex items-center gap-2 px-0.5">
        <h2 className="flex-1 text-ui font-medium text-foreground">Active rules</h2>
        {!writing && <Button variant="outline" size="xs" onClick={() => setWriting(true)}><Plus className="size-3" aria-hidden="true" /> New rule</Button>}
      </div>
      {writing && <div aria-label="New taste rule" role="group" className="mb-3 grid gap-2 rounded-xl border border-border-card bg-card px-4 py-3">
        <SegmentedGroup label="Kind of rule" className="max-w-full">
          {(Object.keys(KIND_LABEL) as Kind[]).map(kind => <SegmentedItem key={kind} active={draft.kind === kind} onClick={() => setDraft(current => ({ ...current, kind }))}>{KIND_LABEL[kind]}</SegmentedItem>)}
        </SegmentedGroup>
        <Textarea aria-label="Rule" rows={2} placeholder="Don't ask for doc comments on private functions." value={draft.body}
          onChange={event => setDraft(current => ({ ...current, body: event.target.value }))} />
        <div className="grid gap-2 sm:grid-cols-2">
          <Input aria-label="Repository" placeholder="owner/repo (empty: every repository)" value={draft.repository}
            onChange={event => setDraft(current => ({ ...current, repository: event.target.value }))} />
          <Input aria-label="Paths" placeholder="src/**, tests/** (empty: every path)" value={draft.paths}
            onChange={event => setDraft(current => ({ ...current, paths: event.target.value }))} />
        </div>
        <div className="flex justify-end gap-1.5">
          <Button variant="quiet" size="xs" onClick={() => setWriting(false)}>Cancel</Button>
          <Button size="xs" disabled={busy || !draft.body.trim()} onClick={() => void save()}>Save rule</Button>
        </div>
      </div>}
      {active.length === 0
        ? <p className="rounded-xl border border-dashed border-border px-4 py-6 text-center text-caption text-muted-foreground">
            No rules yet. As you keep and drop review findings, Bridge proposes rules here; you can also write one.
          </p>
        : <div className="space-y-4">
            {group(active).map(([scope, rules]) => <div key={scope}>
              <p className="mb-1 px-0.5 font-mono text-caption text-muted-foreground">{repositoryLabel(scope)}</p>
              <ul aria-label={`Taste rules for ${repositoryLabel(scope)}`} className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
                {rules.map(rule => <RuleRow key={rule.id} rule={rule} busy={busy} onError={onError}
                  onDelete={() => void act(() => bridgeApi.deleteMemoryRecord(rule.id), "Couldn't delete that rule.")} />)}
              </ul>
            </div>)}
          </div>}
    </section>
  </div>;
}
