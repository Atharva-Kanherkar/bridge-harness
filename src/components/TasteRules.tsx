import { useEffect, useState } from "react";
import { Check, ChevronDown, ChevronRight, FileText, Pause, Pencil, Pin, PinOff, Play, Plus, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SegmentedGroup, SegmentedItem } from "@/components/ui/segmented";
import { SkeletonRows } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { Tip } from "@/components/ui/tooltip";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import type { ExportTasteRulesResult, ImportTasteRulesResult, MemoryRecord, PermissionSuggestion, TasteEvidence } from "../protocol/generated/protocol";

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

/** A rule's trust, signed, one decimal. */
const trust = (score: number) => `${score > 0 ? "+" : score < 0 ? "−" : ""}${Math.abs(score).toFixed(1)}`;

function RuleRow({ rule, busy, showRepository, neverUsed, onApprove, onReject, onDelete, onPause, onResume, onPin, onEdit, onError }: {
  rule: MemoryRecord;
  busy: boolean;
  neverUsed?: boolean;
  onPause?: () => void;
  onResume?: () => void;
  onPin?: (pinned: boolean) => void;
  onEdit?: (body: string) => void;
  /** Name the repository on the row, where rules of several are mixed. */
  showRepository?: boolean;
  onApprove?: () => void;
  onReject?: () => void;
  onDelete?: () => void;
  onError: (message: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<string>();
  const decisions = rule.evidenceCount ?? 0;
  const learned = rule.provenance !== "user_explicit";
  return <li data-taste-rule={rule.id} className="px-4 py-3">
    <div className="flex flex-wrap items-start gap-x-3 gap-y-1">
      <span className="mt-0.5 shrink-0 rounded-sm bg-muted px-1.5 text-caption font-medium text-muted-foreground">{kindLabel(rule.kind)}</span>
      {editing === undefined
        ? <span className="min-w-0 flex-1 basis-64 text-ui text-foreground">{rule.body}</span>
        : <span className="grid min-w-0 flex-1 basis-64 gap-1.5">
            <Textarea aria-label={`Edit rule: ${rule.body}`} rows={2} value={editing} onChange={event => setEditing(event.target.value)} />
            <span className="flex justify-end gap-1.5">
              <Button variant="quiet" size="2xs" onClick={() => setEditing(undefined)}>Cancel</Button>
              <Button size="2xs" disabled={busy || !editing.trim() || editing.trim() === rule.body} onClick={() => { onEdit?.(editing.trim()); setEditing(undefined); }}>Save</Button>
            </span>
          </span>}
      <span className="flex shrink-0 gap-1">
        {onEdit && editing === undefined && <Button variant="ghost" size="2xs" disabled={busy} aria-label={`Edit rule: ${rule.body}`} onClick={() => setEditing(rule.body)}><Pencil className="size-3" aria-hidden="true" /></Button>}
        {onPin && learned && <Button variant="ghost" size="2xs" disabled={busy} aria-pressed={Boolean(rule.pinned)} aria-label={rule.pinned ? `Unpin rule: ${rule.body}` : `Pin rule: ${rule.body}`} onClick={() => onPin(!rule.pinned)}>
          {rule.pinned ? <PinOff className="size-3" aria-hidden="true" /> : <Pin className="size-3" aria-hidden="true" />}
        </Button>}
        {onPause && <Button variant="ghost" size="2xs" disabled={busy} aria-label={`Pause rule: ${rule.body}`} onClick={onPause}><Pause className="size-3" aria-hidden="true" /></Button>}
        {onResume && <Button size="2xs" disabled={busy} onClick={onResume}><Play className="size-3" aria-hidden="true" /> Resume</Button>}
        {onApprove && <Button size="2xs" disabled={busy} onClick={onApprove}><Check className="size-3" aria-hidden="true" /> Approve</Button>}
        {onReject && <Button variant="ghost" size="2xs" disabled={busy} onClick={onReject}><X className="size-3" aria-hidden="true" /> Reject</Button>}
        {onDelete && <Button variant="ghost" size="2xs" disabled={busy} aria-label={`Delete rule: ${rule.body}`} onClick={onDelete}><Trash2 className="size-3" aria-hidden="true" /></Button>}
      </span>
    </div>
    <div className="mt-1 flex flex-wrap items-center gap-x-3 text-caption text-muted-foreground">
      {showRepository && <span className="font-mono">{repositoryLabel(rule.scopeKey)}</span>}
      {rule.pathGlobs?.length ? <span className="font-mono">{rule.pathGlobs.join(", ")}</span> : null}
      {rule.provenance === "user_explicit" ? <span>Written by you</span> : <span>Learned{rule.pinned ? " · pinned" : ""}</span>}
      {learned && rule.status !== "proposed" && <Tip label="Rises when what it shaped is kept, falls when it is dropped; halves every 60 days."><span className="tabular-nums">Trust {trust(rule.outcomeScore ?? 0)}</span></Tip>}
      {neverUsed && <span className="text-warning">Never used</span>}
      {rule.pausedReason && <span>{rule.pausedReason}</span>}
      {rule.rationale && <span>{rule.rationale}</span>}
      {decisions > 0 && <Button variant="text" size="inline" aria-expanded={open} onClick={() => setOpen(current => !current)}>
        {open ? <ChevronDown className="size-3" aria-hidden="true" /> : <ChevronRight className="size-3" aria-hidden="true" />}
        From {decisions} {decisions === 1 ? "decision" : "decisions"}
      </Button>}
    </div>
    {open && <div className="mt-2 rounded-md border border-border bg-background px-3 py-2"><Evidence recordId={rule.id} onError={onError} /></div>}
  </li>;
}

const START = "<!-- bridge:taste:start v1 -->";
const END = "<!-- bridge:taste:end -->";

/** The managed section of a file's text, as export writes it. */
const section = (text: string) => {
  const start = text.indexOf(START);
  const end = text.indexOf(END, start);
  return start < 0 || end < 0 ? "" : text.slice(start, end + END.length);
};

/** A repository's rules out to its instruction files and back. Nothing is
 *  written or applied without a click, and Bridge never commits. */
function TasteFiles({ scopeKey, onChanged, onError }: { scopeKey: string; onChanged: () => void; onError: (message: string) => void }) {
  const [preview, setPreview] = useState<ExportTasteRulesResult>();
  const [plan, setPlan] = useState<ImportTasteRulesResult>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string>();
  const run = async (work: () => Promise<void>, failure: string) => {
    setBusy(true); setDone(undefined);
    try { await work(); } catch (cause) { onError(friendlyError(cause, failure)); } finally { setBusy(false); }
  };
  useEffect(() => {
    let active = true;
    void bridgeApi.exportTasteRules({ scopeKey, write: false })
      .then(found => { if (active) setPreview(found); })
      .catch(cause => { if (active) onError(friendlyError(cause, "Couldn't read this repository's files.")); });
    return () => { active = false; };
  }, [scopeKey, onError]);
  if (!preview) return <SkeletonRows label="Reading the files" rows={2} framed={false} density="menu" />;
  const checkout = preview.checkouts.find(item => item.workspaceId === preview.workspaceId);
  const changed = preview.files.filter(file => file.changed);
  return <div aria-label={`Files for ${repositoryLabel(scopeKey)}`} role="group" className="grid gap-3 rounded-xl border border-border-card bg-card px-4 py-3">
    <p className="text-caption text-muted-foreground">
      In <span className="font-mono text-foreground">{checkout?.path}</span>. Bridge writes only between its markers and never commits.
    </p>
    {preview.files.map(file => <div key={file.path}>
      <p className="mb-1 flex items-center gap-2 text-caption">
        <span className="font-mono text-foreground">{file.path}</span>
        <span className="text-muted-foreground">{file.changed ? (file.before ? "will be updated" : "will be created") : "up to date"}</span>
      </p>
      {file.changed && <pre className="max-h-48 overflow-auto rounded-md border border-border bg-background px-3 py-2 text-caption whitespace-pre-wrap">{section(file.after)}</pre>}
    </div>)}
    <div className="flex flex-wrap items-center justify-end gap-1.5">
      {done && <span role="status" className="mr-auto text-caption text-muted-foreground">{done}</span>}
      <Button variant="outline" size="xs" disabled={busy} onClick={() => void run(async () => {
        setPlan(await bridgeApi.importTasteRules({ scopeKey, workspaceId: preview.workspaceId, apply: false }));
      }, "Couldn't read the files back.")}>Read back from files</Button>
      <Button size="xs" disabled={busy || changed.length === 0} onClick={() => void run(async () => {
        setPreview(await bridgeApi.exportTasteRules({ scopeKey, workspaceId: preview.workspaceId, write: true }));
        setDone(`Wrote ${changed.map(file => file.path).join(", ")}.`);
      }, "Couldn't write the files.")}>Write files</Button>
    </div>
    {plan && <div aria-label="What the files would change" className="grid gap-1 rounded-md border border-border bg-background px-3 py-2 text-caption">
      {plan.added.length + plan.updated.length + plan.missing.length === 0
        ? <p className="text-muted-foreground">The files say what your rules say.</p>
        : <>
            {plan.added.map(body => <p key={`add-${body}`}><span className="font-medium text-foreground">New</span> {body}</p>)}
            {plan.updated.map(update => <p key={`edit-${update.recordId}`}><span className="font-medium text-foreground">Changed</span> {update.body}</p>)}
            {plan.missing.length > 0 && <p className="text-muted-foreground">{plan.missing.length} {plan.missing.length === 1 ? "rule is" : "rules are"} not in the files. Nothing is deleted; pause them here if you meant to.</p>}
            {(plan.added.length > 0 || plan.updated.length > 0) && <div className="flex justify-end">
              <Button size="2xs" disabled={busy} onClick={() => void run(async () => {
                await bridgeApi.importTasteRules({ scopeKey, workspaceId: preview.workspaceId, apply: true });
                setPlan(undefined);
                setDone("Your rules now match the files.");
                onChanged();
              }, "Couldn't apply what the files say.")}>Apply</Button>
            </div>}
          </>}
    </div>}
  </div>;
}

/** Requests you keep answering the same way. Bridge grants nothing from
 *  them: an allow points at chat access, a decline can become a code rule. */
function Suggestions({ items, busy, onDismiss, onRule }: {
  items: PermissionSuggestion[];
  busy: boolean;
  onDismiss: (key: string) => void;
  onRule: (item: PermissionSuggestion) => void;
}) {
  if (items.length === 0) return null;
  return <section>
    <h2 className="mb-2 px-0.5 text-ui font-medium text-foreground">From your approvals</h2>
    <ul aria-label="Suggestions from your approvals" className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
      {items.map(item => {
        const what = item.pattern ? `${item.tool} \`${item.pattern}\`` : item.tool;
        const where = repositoryLabel(item.scopeKey);
        return <li key={item.key} data-suggestion={item.key} className="flex flex-wrap items-start gap-x-3 gap-y-1 px-4 py-3">
          <span className="min-w-0 flex-1 basis-64 text-ui text-foreground">
            You {item.verdict === "allowed" ? "allowed" : "declined"} {what} {item.count} times in {item.sessions} chats ({where}).
            <span className="mt-0.5 block text-caption text-muted-foreground">
              {item.verdict === "allowed"
                ? "To stop being asked, give chats there more access in Settings, Chats. Bridge grants nothing on its own."
                : "Write it down as a code rule, so agents stop asking to run it."}
            </span>
          </span>
          <span className="flex shrink-0 gap-1">
            {item.verdict === "declined" && <Button size="2xs" disabled={busy} onClick={() => onRule(item)}>Make it a rule</Button>}
            <Button variant="ghost" size="2xs" disabled={busy} onClick={() => onDismiss(item.key)}>Dismiss</Button>
          </span>
        </li>;
      })}
    </ul>
  </section>;
}

function group(rules: MemoryRecord[]): [string, MemoryRecord[]][] {
  const byScope = new Map<string, MemoryRecord[]>();
  for (const rule of rules) byScope.set(rule.scopeKey, [...(byScope.get(rule.scopeKey) ?? []), rule]);
  return [...byScope.entries()].sort(([a], [b]) => (a === "account:local" ? 1 : b === "account:local" ? -1 : a.localeCompare(b)));
}

export function TasteRules({ onError, onCount }: { onError: (message: string) => void; onCount?: (waiting: number) => void }) {
  const [waiting, setWaiting] = useState<MemoryRecord[]>();
  const [active, setActive] = useState<MemoryRecord[]>();
  const [paused, setPaused] = useState<MemoryRecord[]>([]);
  const [neverUsed, setNeverUsed] = useState<ReadonlySet<string>>(new Set());
  // The repository whose files panel is open.
  const [files, setFiles] = useState<string>();
  const [suggested, setSuggested] = useState<PermissionSuggestion[]>([]);
  const [busy, setBusy] = useState(false);
  const [writing, setWriting] = useState(false);
  const [draft, setDraft] = useState({ repository: "", kind: "review_avoid" as Kind, body: "", paths: "" });

  const load = async () => {
    const [proposed, current, resting, hints] = await Promise.all([
      bridgeApi.listTasteRules("proposed"), bridgeApi.listTasteRules("active"), bridgeApi.listTasteRules("paused"),
      bridgeApi.listPermissionSuggestions(),
    ]);
    setSuggested(hints.suggestions);
    setWaiting(proposed.rules);
    setActive(current.rules);
    setPaused(resting.rules);
    setNeverUsed(new Set(current.neverUsed ?? []));
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
              <div className="mb-1 flex items-center gap-2 px-0.5">
                <p className="flex-1 font-mono text-caption text-muted-foreground">{repositoryLabel(scope)}</p>
                {scope.startsWith("project:") && <Button variant="quiet" size="2xs" aria-expanded={files === scope}
                  onClick={() => setFiles(current => current === scope ? undefined : scope)}>
                  <FileText className="size-3" aria-hidden="true" /> Files
                </Button>}
              </div>
              {files === scope && <div className="mb-2"><TasteFiles scopeKey={scope} onError={onError} onChanged={() => void load()} /></div>}
              <ul aria-label={`Taste rules for ${repositoryLabel(scope)}`} className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
                {rules.map(rule => <RuleRow key={rule.id} rule={rule} busy={busy} onError={onError} neverUsed={neverUsed.has(rule.id)}
                  onDelete={() => void act(() => bridgeApi.deleteMemoryRecord(rule.id), "Couldn't delete that rule.")}
                  onPause={() => void act(() => bridgeApi.setTasteRuleState(rule.id, { state: "paused" }), "Couldn't pause that rule.")}
                  onPin={pinned => void act(() => bridgeApi.setTasteRuleState(rule.id, { pinned }), "Couldn't pin that rule.")}
                  onEdit={body => void act(() => bridgeApi.supersedeMemoryRecord(rule.id, body), "Couldn't edit that rule.")} />)}
              </ul>
            </div>)}
          </div>}
    </section>
    <Suggestions items={suggested} busy={busy}
      onDismiss={key => void act(() => bridgeApi.dismissPermissionSuggestion(key), "Couldn't dismiss that suggestion.")}
      onRule={item => void act(async () => {
        await bridgeApi.addTasteRule({
          scopeKey: item.scopeKey.startsWith("project:") ? item.scopeKey : "account:local",
          kind: "code_prefer",
          body: `Don't run \`${item.pattern || item.tool}\` without asking me first.`,
        });
        await bridgeApi.dismissPermissionSuggestion(item.key);
      }, "Couldn't write that rule.")} />
    {paused.length > 0 && <section>
      <h2 className="mb-2 px-0.5 text-ui font-medium text-foreground">Paused <span className="tabular-nums text-faint">{paused.length}</span></h2>
      <ul aria-label="Paused taste rules" className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
        {paused.map(rule => <RuleRow key={rule.id} rule={rule} busy={busy} onError={onError} showRepository
          onResume={() => void act(() => bridgeApi.setTasteRuleState(rule.id, { state: "active" }), "Couldn't resume that rule.")}
          onDelete={() => void act(() => bridgeApi.deleteMemoryRecord(rule.id), "Couldn't delete that rule.")} />)}
      </ul>
      <p className="mt-2 px-0.5 text-caption text-faint">A learned rule pauses itself when what it shapes keeps getting dropped. Paused rules reach no review and no chat.</p>
    </section>}
  </div>;
}
