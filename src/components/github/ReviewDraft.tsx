import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Pencil, Send, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../../api";
import type { ReviewFinding, ReviewFindingDecisionInput, ReviewJob, ReviewPass, ReviewSeverity } from "../../protocol/generated/protocol";
import { GithubMarkdown } from "./GithubMarkdown";
import { harnessLabel, SEVERITY_TEXT } from "./reviewLabels";

// A drafted review, for a person to triage: keep, drop or edit each finding,
// then post the kept ones as one comment review. Nothing reaches GitHub until
// Post. Keyboard: arrows move between findings, K keeps, D drops, E edits.

const SEVERITY_RANK: Record<ReviewSeverity, number> = { critical: 0, major: 1, minor: 2, nit: 3 };
const OUTSIDE = "Outside the diff";

type Group = { key: string; label: string; findings: ReviewFinding[] };

/** Inline findings by file (path order), then the ones outside the diff;
 *  in each group by severity, then confidence. */
export function groupFindings(findings: ReviewFinding[]): Group[] {
  const order = (a: ReviewFinding, b: ReviewFinding) =>
    SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity] || b.confidence - a.confidence;
  const byPath = new Map<string, ReviewFinding[]>();
  const outside: ReviewFinding[] = [];
  for (const finding of findings) {
    if (finding.outsideDiff) { outside.push(finding); continue; }
    byPath.set(finding.path, [...(byPath.get(finding.path) ?? []), finding]);
  }
  const groups: Group[] = [...byPath.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([path, items]) => ({ key: path, label: path, findings: items.sort(order) }));
  if (outside.length) groups.push({ key: OUTSIDE, label: OUTSIDE, findings: outside.sort(order) });
  return groups;
}

const where = (finding: ReviewFinding) => {
  const lines = finding.startLine && finding.startLine < finding.line ? `${finding.startLine}–${finding.line}` : `${finding.line}`;
  return `${finding.path}:${lines}${finding.side === "LEFT" ? " (old)" : ""}`;
};

const chosen = (finding: ReviewFinding) => finding.decision === "kept" || finding.decision === "edited";

/** What a finding says now: the person's saved edit, else the reviewer's body. */
const savedBody = (finding: ReviewFinding) => finding.editedBody ?? finding.body;

function FindingItem({ finding, harness, repositoryUrl, draft, busy, onDraft, onFocusMove, onDecide, onEdit, onCancelEdit, register }: {
  finding: ReviewFinding;
  harness?: string;
  repositoryUrl?: string;
  /** The open editor's text; absent when the finding is not being edited. */
  draft?: string;
  busy: boolean;
  onDraft: (text: string) => void;
  onFocusMove: (step: number) => void;
  onDecide: (decision: ReviewFindingDecisionInput["decision"], editedBody?: string) => void;
  onEdit: () => void;
  onCancelEdit: () => void;
  register: (element: HTMLElement | null) => void;
}) {
  const editing = draft !== undefined;
  const text = draft ?? "";
  const dropped = finding.decision === "dropped";
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || event.metaKey || event.ctrlKey || event.altKey) return;
    const key = event.key.toLowerCase();
    if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); onFocusMove(event.key === "ArrowDown" ? 1 : -1); }
    // Mid-edit, a decision would close the editor and lose the text.
    else if (key === "k" && !busy && !editing) { event.preventDefault(); onDecide("kept"); }
    else if (key === "d" && !busy && !editing) { event.preventDefault(); onDecide("dropped"); }
    else if (key === "e") { event.preventDefault(); onEdit(); }
  };
  return <li
    ref={register}
    tabIndex={0}
    data-finding={finding.id}
    data-decision={finding.decision}
    aria-label={`${finding.severity} finding: ${finding.title}, ${finding.decision}`}
    onKeyDown={onKeyDown}
    className={cn("rounded-md border px-3 py-2 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
      chosen(finding) ? "border-success/40 bg-success/5" : "border-border bg-background", dropped && "opacity-60")}
  >
    <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
      <span className={cn("text-caption font-semibold", SEVERITY_TEXT[finding.severity])}>{finding.severity.charAt(0).toUpperCase() + finding.severity.slice(1)}</span>
      <span className={cn("min-w-0 flex-1 text-meta font-medium text-foreground", dropped && "line-through")}>{finding.title}</span>
      {finding.agreedBy > 1 && <span className="rounded-sm bg-info/10 px-1.5 text-caption font-medium text-info">Both reviewers</span>}
    </div>
    <div className="mt-0.5 flex flex-wrap items-center gap-x-2 text-caption text-muted-foreground">
      <span className="font-mono">{where(finding)}</span>
      <span>{Math.round(finding.confidence * 100)}% sure</span>
      {harness && <span>{harnessLabel(harness)}</span>}
      {finding.decision !== "pending" && <span className="font-medium text-foreground">{finding.decision === "kept" ? "Kept" : finding.decision === "edited" ? "Edited" : "Dropped"}</span>}
    </div>
    {editing
      ? <div className="mt-2 grid gap-1.5">
          <textarea
            aria-label={`Edit finding: ${finding.title}`}
            autoFocus
            value={text}
            rows={5}
            onChange={event => onDraft(event.target.value)}
            onKeyDown={event => {
              if (event.key === "Escape") { event.preventDefault(); onCancelEdit(); }
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && text.trim()) { event.preventDefault(); onDecide("edited", text); }
            }}
            className="w-full rounded-md border border-input bg-background px-2 py-1.5 text-meta text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <div className="flex justify-end gap-1.5">
            <button type="button" onClick={onCancelEdit} className="rounded-md px-2 py-0.5 text-meta text-muted-foreground hover:bg-accent">Cancel</button>
            <button type="button" disabled={busy || !text.trim()} onClick={() => onDecide("edited", text)} className="rounded-md bg-primary px-2 py-0.5 text-meta font-medium text-primary-foreground disabled:opacity-50">Save edit</button>
          </div>
        </div>
      : <div className="mt-1.5 text-meta text-foreground/90"><GithubMarkdown text={savedBody(finding)} repositoryUrl={repositoryUrl} /></div>}
    {!editing && <div className="mt-1.5 flex gap-1">
      <button type="button" disabled={busy} aria-pressed={finding.decision === "kept"} onClick={() => onDecide("kept")} className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-foreground hover:bg-accent disabled:opacity-50"><Check size={11} aria-hidden="true" /> Keep</button>
      <button type="button" disabled={busy} aria-pressed={dropped} onClick={() => onDecide("dropped")} className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-foreground hover:bg-accent disabled:opacity-50"><X size={11} aria-hidden="true" /> Drop</button>
      <button type="button" disabled={busy} onClick={onEdit} className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-foreground hover:bg-accent disabled:opacity-50"><Pencil size={11} aria-hidden="true" /> Edit</button>
    </div>}
  </li>;
}

export function ReviewDraft({ job, repositoryUrl, onPosting }: {
  job: ReviewJob;
  repositoryUrl?: string;
  /** The job as `reviews/post_review` returned it (now `posting`). */
  onPosting: (job: ReviewJob) => void;
}) {
  const [findings, setFindings] = useState<ReviewFinding[]>();
  const [passes, setPasses] = useState<ReviewPass[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // Open editors by finding id. The text lives here, not in the item, so
  // Post can see an edit that is not saved yet, and deciding another
  // finding leaves it alone.
  const [drafts, setDrafts] = useState<ReadonlyMap<string, string>>(new Map());
  const setDraft = (id: string, text: string | undefined) => setDrafts(current => {
    const next = new Map(current);
    if (text === undefined) next.delete(id); else next.set(id, text);
    return next;
  });
  const items = useRef(new Map<string, HTMLElement>());

  useEffect(() => {
    let active = true;
    setFindings(undefined); setError(undefined); setDrafts(new Map());
    void bridgeApi.getReview(job.id)
      .then(detail => { if (active) { setFindings(detail.findings); setPasses(detail.passes); } })
      .catch(cause => { if (active) setError(cause instanceof Error ? cause.message : String(cause)); });
    return () => { active = false; };
  }, [job.id]);

  const groups = useMemo(() => groupFindings(findings ?? []), [findings]);
  const flat = useMemo(() => groups.flatMap(group => group.findings), [groups]);
  const harnessOf = useMemo(() => new Map(passes.map(pass => [pass.id, pass.harness])), [passes]);

  const decide = async (decisions: ReviewFindingDecisionInput[]) => {
    if (!decisions.length) return;
    setBusy(true); setError(undefined);
    try {
      const result = await bridgeApi.decideReview(job.id, decisions);
      setFindings(result.findings);
      // A decided finding's editor closes; any other stays open.
      setDrafts(current => {
        const next = new Map(current);
        for (const decision of decisions) next.delete(decision.findingId);
        return next;
      });
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  const post = async () => {
    setBusy(true); setError(undefined);
    try { onPosting((await bridgeApi.postReview(job.id)).job); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); setBusy(false); }
  };
  const move = (id: string, step: number) => {
    const index = flat.findIndex(finding => finding.id === id);
    const next = flat[Math.min(flat.length - 1, Math.max(0, index + step))];
    if (next) items.current.get(next.id)?.focus();
  };

  if (!findings) return <div className="border-t border-border/70 px-3 py-2">
    {error ? <p role="alert" className="text-meta text-destructive">{error}</p> : <p className="text-meta text-muted-foreground">Loading the draft…</p>}
  </div>;
  // Keep all and Drop all pass over a finding being edited.
  const settled = findings.filter(finding => !drafts.has(finding.id));
  const count = findings.filter(chosen).length;
  const undecided = findings.filter(finding => finding.decision === "pending").length;
  const unsaved = findings.some(finding => drafts.has(finding.id) && drafts.get(finding.id) !== savedBody(finding));
  const blocked = unsaved || (findings.length > 0 && count === 0);
  const reason = unsaved ? "Save or cancel your edit to post." : blocked ? "Keep or edit at least one finding to post." : undefined;
  const label = findings.length === 0 ? "Post summary" : count === 0 ? "Post" : `Post ${count} ${count === 1 ? "finding" : "findings"}`;
  return <div data-review-draft={job.id} className="border-t border-border/70 bg-muted/20 px-3 py-2.5">
    {findings.length > 0 && <div className="mb-2 flex flex-wrap items-center gap-1.5">
      <button type="button" disabled={busy} onClick={() => void decide(settled.map(finding => ({ findingId: finding.id, decision: "kept" })))} className="rounded-md border border-border bg-card px-2 py-0.5 text-meta text-foreground hover:bg-accent disabled:opacity-50">Keep all</button>
      <button type="button" disabled={busy} onClick={() => void decide(settled.map(finding => ({ findingId: finding.id, decision: "dropped" })))} className="rounded-md border border-border bg-card px-2 py-0.5 text-meta text-foreground hover:bg-accent disabled:opacity-50">Drop all</button>
      <span className="ml-auto text-caption text-muted-foreground"><kbd>K</kbd> keep · <kbd>D</kbd> drop · <kbd>E</kbd> edit · arrows move</span>
    </div>}
    {findings.length === 0 && <p className="mb-2 text-meta text-muted-foreground">The reviewer found nothing to flag. Posting leaves its summary as a comment review.</p>}
    {groups.map(group => <div key={group.key} className="mb-2.5">
      <p className={cn("mb-1 text-caption font-medium text-muted-foreground", group.key !== OUTSIDE && "font-mono")}>{group.label}</p>
      <ul role="list" aria-label={`Findings in ${group.label}`} className="grid gap-1.5">
        {group.findings.map(finding => <FindingItem
          key={finding.id}
          finding={finding}
          harness={harnessOf.get(finding.passId)}
          repositoryUrl={repositoryUrl}
          draft={drafts.get(finding.id)}
          busy={busy}
          onDraft={text => setDraft(finding.id, text)}
          register={element => { if (element) items.current.set(finding.id, element); else items.current.delete(finding.id); }}
          onFocusMove={step => move(finding.id, step)}
          onDecide={(decision, editedBody) => void decide([{ findingId: finding.id, decision, editedBody: decision === "edited" ? editedBody : null }])}
          onEdit={() => { if (!drafts.has(finding.id)) setDraft(finding.id, savedBody(finding)); }}
          onCancelEdit={() => { setDraft(finding.id, undefined); items.current.get(finding.id)?.focus(); }}
        />)}
      </ul>
    </div>)}
    {error && <p role="alert" className="mb-2 text-meta text-destructive">{error}</p>}
    <div className="flex items-center justify-end gap-2">
      {reason && <span className="text-caption text-muted-foreground">{reason}</span>}
      {!reason && undecided > 0 && <span className="text-caption text-muted-foreground">{count} kept, {undecided} not decided</span>}
      <button type="button" disabled={busy || blocked} onClick={() => void post()} className="inline-flex min-h-7 items-center gap-1.5 rounded-md bg-primary px-3 py-1 text-meta font-medium text-primary-foreground shadow-xs hover:bg-primary/90 disabled:opacity-50">
        <Send size={11} aria-hidden="true" /> {label}
      </button>
    </div>
  </div>;
}
