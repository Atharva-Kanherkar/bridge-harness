import { useState } from "react";
import { FolderGit2, GitBranch } from "lucide-react";
import type { Workspace } from "../types";
import { TerminalWorkspace } from "./TerminalWorkspace";

const SELECTION_KEY = "bridge.agent-fleet.workspace";
export function AgentFleet({ workspaces, initialWorkspaceId, onOpenProjects, onError }: {
  workspaces: Workspace[];
  initialWorkspaceId?: string | null;
  onOpenProjects: () => void;
  onError: (message: string) => void;
}) {
  const [selected, setSelected] = useState(() => localStorage.getItem(SELECTION_KEY) ?? initialWorkspaceId ?? "");
  const available = workspaces;
  const workspace = available.find(w => w.id === selected) ?? available.find(w => w.id === initialWorkspaceId) ?? available[0];
  return <main className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background" aria-label="Agent Fleet">
    <div className="flex shrink-0 flex-wrap items-center gap-3 border-b border-border px-4 py-3">
      <FolderGit2 size={15} className="text-muted-foreground" aria-hidden="true" />
      <label className="sr-only" htmlFor="mission-workspace">Agent Fleet workspace</label>
      <select id="mission-workspace" value={workspace?.id ?? ""} onChange={event => { setSelected(event.target.value); localStorage.setItem(SELECTION_KEY, event.target.value); }} className="max-w-full min-w-0 flex-1 truncate rounded-md bg-transparent py-1 text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring">
        {!available.length && <option value="">No workspace connected</option>}
        {available.map(w => <option key={w.id} value={w.id}>{w.title}</option>)}
      </select>
      {workspace && <span className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground"><GitBranch size={12} />{workspace.branch}</span>}
    </div>
    {workspace ? <TerminalWorkspace key={workspace.id} workspaceId={workspace.id} branch={workspace.branch ?? ""} onError={onError} /> : <div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center"><h2 className="font-display text-xl">Connect a workspace to get started</h2><p className="max-w-sm text-sm text-muted-foreground">Agent Fleet opens your terminals and agent CLIs in the checkout you choose.</p><button type="button" onClick={onOpenProjects} className="rounded-md border border-border bg-card px-4 py-2 text-xs font-medium hover:bg-accent">Open Projects</button></div>}
  </main>;
}
