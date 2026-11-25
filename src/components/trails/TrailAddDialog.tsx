import { useEffect, useRef, useState, type ComponentType, type FormEvent } from "react";
import { CircleDot, FileText, GitPullRequest, MessagesSquare, Plus, Route, Search, Send } from "lucide-react";
import type { TrailCandidate, TrailMemberKind, TrailMemberRef, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { HarnessMark } from "@/components/harnessMarks";
import { SkeletonText } from "@/components/ui/skeleton";
import { whenLabel } from "@/trails";
import { clsx as cn } from "clsx";
import { RowButton } from "@/components/ui/row";

const KINDS: { kind: TrailMemberKind; label: string; plural: string; icon: ComponentType<{ className?: string }> }[] = [
  { kind: "note", label: "Note", plural: "notes", icon: FileText },
  { kind: "session", label: "Chat", plural: "chats", icon: MessagesSquare },
  { kind: "pullRequest", label: "Pull request", plural: "pull requests", icon: GitPullRequest },
  { kind: "outgoing", label: "Draft", plural: "drafts", icon: Send },
  { kind: "issue", label: "Issue", plural: "issues", icon: CircleDot },
];

const INPUT = "h-8 min-w-0 flex-1 rounded-lg border border-input bg-background px-3 text-ui text-foreground outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-ring";

function message(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/** One field: the issue's link, `owner/repo#12`, or `#12` when the trail has
 * a project. The core reads it from GitHub; a refusal shows here. */
function AddIssueForm({ hasProject, onAdd }: { hasProject: boolean; onAdd: (reference: string) => Promise<void> }) {
  const [reference, setReference] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!reference.trim() || busy) return;
    setBusy(true);
    setError(undefined);
    try { await onAdd(reference.trim()); setReference(""); }
    catch (cause) { setError(message(cause)); }
    finally { setBusy(false); }
  }
  return (
    <form onSubmit={submit} aria-label="Add issue">
      <div className="flex items-center gap-2">
        <input
          autoFocus
          value={reference}
          onChange={event => setReference(event.target.value)}
          placeholder={hasProject ? "Issue link, owner/repo#12, or #12" : "Issue link or owner/repo#12"}
          aria-label="Issue"
          className={INPUT}
        />
        <Button type="submit" size="sm" disabled={!reference.trim() || busy}>{busy ? "Reading…" : "Add"}</Button>
      </div>
      <p className="mt-2 text-caption text-muted-foreground">Bridge reads the issue from GitHub, and keeps its pull requests in step.</p>
      {error && <p role="alert" className="mt-2 text-caption text-destructive">{error}</p>}
    </form>
  );
}

/** A title, then a page on the trail, opened in Notes. */
function NewNoteForm({ onCreate }: { onCreate: (title: string) => Promise<void> }) {
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!title.trim() || busy) return;
    setBusy(true);
    setError(undefined);
    try { await onCreate(title.trim()); }
    catch (cause) { setError(message(cause)); setBusy(false); }
  }
  return (
    <form onSubmit={submit} aria-label="New note on this trail" className="rounded-xl bg-card p-3">
      <div className="flex items-center gap-2">
        <input value={title} maxLength={120} onChange={event => setTitle(event.target.value)} placeholder="New note title" aria-label="New note title" className={INPUT} />
        <Button type="submit" size="sm" disabled={!title.trim() || busy}><Plus aria-hidden="true" />New note</Button>
      </div>
      <p className="mt-2 text-caption text-muted-foreground">Makes a page on this trail and opens it in Notes.</p>
      {error && <p role="alert" className="mt-2 text-caption text-destructive">{error}</p>}
    </form>
  );
}

function CandidateRow({ candidate, icon: Icon, busy, onPick }: { candidate: TrailCandidate; icon: ComponentType<{ className?: string }>; busy: boolean; onPick: () => void }) {
  const elsewhere = candidate.onTrailTitle ?? (candidate.onTrailId ? "another trail" : undefined);
  return (
    <li>
      <RowButton size="bare" tone="default" shape="card"
       
        disabled={busy || Boolean(elsewhere)}
        onClick={onPick}
        aria-label={elsewhere ? `${candidate.title}, on the trail “${elsewhere}”` : `Add ${candidate.title}`}><span className="group flex w-full items-center gap-3 px-3 py-2">
        <span className="flex size-5 shrink-0 items-center justify-center">
          {candidate.harness ? <HarnessMark harness={candidate.harness} size={13} /> : <Icon className="size-3.5 text-muted-foreground" />}
        </span>
        <span className="min-w-0 flex-1">
          <span className={cn("block truncate text-ui", elsewhere ? "text-muted-foreground" : "text-foreground")}>{candidate.title}</span>
          <span className="block truncate text-caption text-faint">
            {elsewhere ? `On “${elsewhere}”` : [candidate.detail, whenLabel(candidate.at)].filter(Boolean).join(" · ")}
          </span>
        </span>
        {!elsewhere && <Plus className="size-3.5 shrink-0 text-faint opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" aria-hidden="true" />}
      </span></RowButton>
    </li>
  );
}

/** Put a note, chat, pull request, draft or issue on the trail by hand, or
 * start a new note on it. Stays open so several can go on in a row. */
export function TrailAddDialog({ trail, initialKind = "note", onAdd, onAddIssue, onCreateNote, onClose }: {
  trail: TrailSummary;
  initialKind?: TrailMemberKind;
  /** Rejects with the core's reason, shown in the dialog. */
  onAdd: (member: TrailMemberRef) => Promise<void>;
  onAddIssue: (reference: string) => Promise<void>;
  /** Make a page on the trail and open it; the dialog closes. */
  onCreateNote: (title: string) => Promise<void>;
  onClose: () => void;
}) {
  const [kind, setKind] = useState<TrailMemberKind>(initialKind);
  const [query, setQuery] = useState("");
  const [candidates, setCandidates] = useState<TrailCandidate[] | null>(null);
  const [error, setError] = useState<string>();
  const [added, setAdded] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const request = useRef(0);
  const meta = KINDS.find(item => item.kind === kind)!;

  useEffect(() => {
    if (kind === "issue") return;
    const id = ++request.current;
    setCandidates(null);
    const timer = window.setTimeout(() => {
      bridgeApi.listTrailCandidates(trail.id, kind, query)
        .then(result => { if (id === request.current) { setCandidates(result.candidates); setError(undefined); } })
        .catch(cause => { if (id === request.current) { setCandidates([]); setError(message(cause)); } });
    }, query ? 150 : 0);
    return () => window.clearTimeout(timer);
  }, [trail.id, kind, query, reload]);

  function choose(next: TrailMemberKind) {
    if (next === kind) return;
    setKind(next);
    setQuery("");
    setError(undefined);
  }

  async function pick(candidate: TrailCandidate) {
    if (busy) return;
    setBusy(true);
    setError(undefined);
    try {
      await onAdd(candidate.member);
      setAdded(`Added “${candidate.title}”.`);
      setReload(value => value + 1);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={next => { if (!next) onClose(); }}>
      <DialogPopup aria-label={`Add to ${trail.title}`} className="max-w-lg">
        <div className="flex min-h-0 flex-col gap-4 p-5">
          <div className="flex items-center gap-2 pr-8">
            <Route className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <h2 className="truncate text-ui font-medium text-foreground">Add to “{trail.title}”</h2>
          </div>
          <div role="tablist" aria-label="What to add" className="flex flex-wrap gap-1">
            {KINDS.map(item => (
              <button
                key={item.kind}
                type="button"
                role="tab"
                aria-selected={kind === item.kind}
                onClick={() => choose(item.kind)}
                className={cn(
                  "inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-caption font-medium transition-colors",
                  kind === item.kind ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/60",
                )}
              >
                <item.icon className="size-3" aria-hidden="true" />{item.label}
              </button>
            ))}
          </div>

          <div role="tabpanel" aria-label={meta.label} className="flex min-h-80 flex-col gap-3">
            {kind === "issue" ? (
              <AddIssueForm hasProject={Boolean(trail.workspaceId)} onAdd={async reference => { await onAddIssue(reference); setAdded("Added the issue."); }} />
            ) : (
              <>
                {kind === "note" && <NewNoteForm onCreate={onCreateNote} />}
                <label className="flex items-center gap-2 rounded-lg border border-input bg-background px-3 focus-within:ring-2 focus-within:ring-ring">
                  <Search className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
                  <input
                    autoFocus={kind !== "note"}
                    value={query}
                    onChange={event => setQuery(event.target.value)}
                    placeholder={`Search ${meta.plural}`}
                    aria-label={`Search ${meta.plural}`}
                    className="h-8 min-w-0 flex-1 bg-transparent text-ui text-foreground outline-none placeholder:text-faint"
                  />
                </label>
                {!candidates && <SkeletonText lines={4} />}
                {candidates && candidates.length === 0 && !error && (
                  <p className="px-1 py-2 text-ui text-muted-foreground">{query.trim() ? `No ${meta.plural} match “${query.trim()}”.` : `No ${meta.plural} to add.`}</p>
                )}
                {candidates && candidates.length > 0 && (
                  <ul className="-mx-2 max-h-72 overflow-y-auto" aria-label={meta.plural}>
                    {candidates.map(candidate => (
                      <CandidateRow key={`${candidate.member.kind}:${candidate.member.ref}`} candidate={candidate} icon={meta.icon} busy={busy} onPick={() => void pick(candidate)} />
                    ))}
                  </ul>
                )}
              </>
            )}
          </div>
          {added && <p role="status" className="text-caption text-muted-foreground">{added}</p>}
          {error && <p role="alert" className="text-caption text-destructive">{error}</p>}
        </div>
      </DialogPopup>
    </Dialog>
  );
}
