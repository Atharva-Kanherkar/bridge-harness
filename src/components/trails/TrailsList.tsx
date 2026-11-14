import { useState, type FormEvent } from "react";
import { Plus, Route } from "lucide-react";
import type { TrailSummary } from "@/protocol/generated/protocol";
import { groupTrails, stageLabel } from "@/trails";
import { HarnessMark } from "@/components/harnessMarks";
import { Button } from "@/components/ui/button";
import { ScreenHeading } from "@/components/ui/screen";
import { SkeletonRows } from "@/components/ui/skeleton";
import { StageBar, StateWord } from "./TrailParts";
import { clsx as cn } from "clsx";

export type TrailWorkspace = { id: string; title: string };

function relative(at: string, now = Date.now()): string {
  const then = new Date(at).getTime();
  if (Number.isNaN(then)) return "";
  const minutes = Math.round((now - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

function NewTrailForm({ workspaces, defaultWorkspaceId, onCreate, onCancel }: {
  workspaces: TrailWorkspace[];
  defaultWorkspaceId?: string | null;
  onCreate: (title: string, workspaceId: string | null) => Promise<void>;
  onCancel: () => void;
}) {
  const [title, setTitle] = useState("");
  const [workspaceId, setWorkspaceId] = useState<string>(defaultWorkspaceId ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!title.trim() || busy) return;
    setBusy(true); setError(undefined);
    try { await onCreate(title.trim(), workspaceId || null); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  }
  return (
    <form onSubmit={submit} className="mb-5 rounded-xl bg-card p-4" aria-label="New trail">
      <div className="flex flex-wrap items-center gap-2">
        <input
          autoFocus
          value={title}
          onChange={event => setTitle(event.target.value)}
          onKeyDown={event => { if (event.key === "Escape") onCancel(); }}
          maxLength={120}
          placeholder="What is this piece of work?"
          aria-label="Trail title"
          className="h-8 min-w-0 flex-1 rounded-lg border border-input bg-background px-3 text-ui text-foreground outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-ring"
        />
        <select
          value={workspaceId}
          onChange={event => setWorkspaceId(event.target.value)}
          aria-label="Project"
          className="h-8 max-w-48 rounded-lg border border-input bg-background px-2 text-ui text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <option value="">No project</option>
          {workspaces.map(workspace => <option key={workspace.id} value={workspace.id}>{workspace.title}</option>)}
        </select>
        <Button type="submit" size="sm" disabled={!title.trim() || busy}>Create</Button>
        <Button type="button" size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
      </div>
      {error && <p role="alert" className="mt-2 text-caption text-destructive">{error}</p>}
    </form>
  );
}

function TrailRow({ trail, workspaceTitle, onOpen }: { trail: TrailSummary; workspaceTitle?: string; onOpen: () => void }) {
  const last = trail.needsYouReason ?? (trail.lastEvent ? `${trail.lastEvent.title} · ${relative(trail.lastEvent.at)}` : "Nothing on it yet");
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        className="flex w-full items-center gap-4 rounded-xl px-4 py-3 text-left transition-colors hover:bg-card focus-visible:bg-card focus-visible:outline-none"
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate text-ui font-medium text-foreground">{trail.title}</span>
          <span className="mt-0.5 block truncate text-caption text-muted-foreground">
            {workspaceTitle ? `${workspaceTitle} · ` : ""}{last}
          </span>
        </span>
        <span className="hidden items-center gap-1 sm:flex" aria-hidden="true">
          {trail.harnesses.map(item => <HarnessMark key={item.harness} harness={item.harness} size={13} />)}
        </span>
        <span className="hidden w-36 flex-col gap-1 md:flex">
          <StageBar stage={trail.stage} state={trail.state} />
          <span className="text-caption text-faint">{stageLabel(trail.stage)}</span>
        </span>
        <StateWord state={trail.state} className="w-24 text-right" />
      </button>
    </li>
  );
}

export function TrailsList({ trails, loading, error, workspaces, defaultWorkspaceId, showArchived, onToggleArchived, onOpen, onCreate }: {
  trails: TrailSummary[] | null;
  loading: boolean;
  error?: string;
  workspaces: TrailWorkspace[];
  defaultWorkspaceId?: string | null;
  showArchived: boolean;
  onToggleArchived: () => void;
  onOpen: (trailId: string) => void;
  onCreate: (title: string, workspaceId: string | null) => Promise<void>;
}) {
  const [creating, setCreating] = useState(false);
  const titleOf = (id?: string | null) => workspaces.find(workspace => workspace.id === id)?.title;
  const sections = trails ? groupTrails(trails) : [];
  const archived = trails?.filter(trail => trail.archivedAt) ?? [];
  return (
    <div className="mx-auto w-full max-w-page px-5 pb-12 pt-6 sm:px-8">
      <ScreenHeading
        title="Trails"
        description="One page per piece of work: its plan, chats, pull requests and messages, and what waits on you."
        action={<Button size="sm" onClick={() => setCreating(true)}><Plus aria-hidden="true" />New trail</Button>}
      />
      {creating && (
        <NewTrailForm
          workspaces={workspaces}
          defaultWorkspaceId={defaultWorkspaceId}
          onCancel={() => setCreating(false)}
          onCreate={async (title, workspaceId) => { await onCreate(title, workspaceId); setCreating(false); }}
        />
      )}
      {error && <p role="alert" className="mb-4 rounded-lg bg-destructive/8 px-3 py-2 text-ui text-destructive">{error}</p>}
      {!trails && loading && <SkeletonRows label="Loading trails" rows={4} />}
      {trails && sections.length === 0 && !creating && (
        <div className="flex flex-col items-center gap-3 rounded-xl bg-card px-6 py-12 text-center">
          <Route className="size-5 text-muted-foreground" aria-hidden="true" />
          <p className="max-w-md text-ui text-muted-foreground">
            No trails yet. Start one for a piece of work, or add a chat to a trail from its menu. Its pull requests, drafts and notes follow it.
          </p>
          <Button size="sm" onClick={() => setCreating(true)}><Plus aria-hidden="true" />New trail</Button>
        </div>
      )}
      {sections.map(section => (
        <section key={section.state} className="mb-5" aria-label={section.label}>
          <h2 className="mb-1 px-4 text-caption font-medium uppercase tracking-wide text-faint">
            {section.label} · {section.trails.length}
          </h2>
          <ul>
            {section.trails.map(trail => <TrailRow key={trail.id} trail={trail} workspaceTitle={titleOf(trail.workspaceId)} onOpen={() => onOpen(trail.id)} />)}
          </ul>
        </section>
      ))}
      {trails && (
        <div className="mt-6 px-4">
          <button type="button" onClick={onToggleArchived} className={cn("text-caption hover:text-foreground", showArchived ? "text-foreground" : "text-muted-foreground")} aria-pressed={showArchived}>
            {showArchived ? "Hide archived" : "Show archived"}
          </button>
          {showArchived && (
            <ul className="mt-2">
              {archived.length === 0 && <li className="px-4 py-2 text-caption text-faint">No archived trails.</li>}
              {archived.map(trail => <TrailRow key={trail.id} trail={trail} workspaceTitle={titleOf(trail.workspaceId)} onOpen={() => onOpen(trail.id)} />)}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
