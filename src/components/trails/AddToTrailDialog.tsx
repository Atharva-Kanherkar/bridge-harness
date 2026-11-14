import { useState, type FormEvent } from "react";
import { Plus, Route } from "lucide-react";
import type { TrailSummary } from "@/protocol/generated/protocol";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { StageBar } from "./TrailParts";

/** Put a chat on an open trail, or start a new trail from it. */
export function AddToTrailDialog({ chatTitle, trails, onAdd, onCreate, onClose }: {
  chatTitle: string;
  trails: TrailSummary[];
  onAdd: (trailId: string) => Promise<void>;
  onCreate: (title: string) => Promise<void>;
  onClose: () => void;
}) {
  const [title, setTitle] = useState(chatTitle);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const open = trails.filter(trail => !trail.archivedAt);

  async function run(action: () => Promise<void>) {
    if (busy) return;
    setBusy(true); setError(undefined);
    try { await action(); onClose(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); setBusy(false); }
  }

  function create(event: FormEvent) {
    event.preventDefault();
    if (title.trim()) void run(() => onCreate(title.trim()));
  }

  return (
    <Dialog open onOpenChange={next => { if (!next) onClose(); }}>
      <DialogPopup aria-label="Add to trail" className="max-w-md">
        <div className="flex flex-col gap-4 p-5">
          <div className="flex items-center gap-2 pr-8">
            <Route className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <h2 className="truncate text-ui font-medium text-foreground">Add “{chatTitle}” to a trail</h2>
          </div>
          {open.length > 0 && (
            <ul className="max-h-64 overflow-y-auto" aria-label="Open trails">
              {open.map(trail => (
                <li key={trail.id}>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void run(() => onAdd(trail.id))}
                    className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left hover:bg-card focus-visible:bg-card focus-visible:outline-none"
                  >
                    <span className="min-w-0 flex-1 truncate text-ui text-foreground">{trail.title}</span>
                    <StageBar stage={trail.stage} state={trail.state} />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <form onSubmit={create} className="flex flex-col gap-2 rounded-xl bg-card p-3" aria-label="New trail from this chat">
            <span className="text-caption font-medium uppercase tracking-wide text-faint">New trail from this chat</span>
            <div className="flex gap-2">
              <input
                value={title}
                maxLength={120}
                onChange={event => setTitle(event.target.value)}
                aria-label="Trail title"
                className="h-8 min-w-0 flex-1 rounded-lg border border-input bg-background px-3 text-ui text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
              />
              <Button type="submit" size="sm" disabled={busy || !title.trim()}><Plus aria-hidden="true" />Create</Button>
            </div>
            <p className="text-caption text-muted-foreground">Its pull requests, drafts, notes and forks join the trail with it.</p>
          </form>
          {error && <p role="alert" className="text-caption text-destructive">{error}</p>}
        </div>
      </DialogPopup>
    </Dialog>
  );
}
