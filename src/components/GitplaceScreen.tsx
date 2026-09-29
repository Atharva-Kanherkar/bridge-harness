import { useMemo, useState } from "react";
import { Check, ChevronDown, FolderGit2 } from "lucide-react";
import { MenuItem, MenuPanel, useMenuPanel } from "@/components/ui/menu-panel";
import { cn } from "@/lib/utils";
import { GitHubPane } from "./GitHubPane";
import { Switch } from "./settings/kit";
import { useAttributionSettings } from "../attributionSettings";
import type { Project, Workspace } from "../types";

// GitHub as a place of its own. The dock pane only exists inside a chat, so
// with no chat open there was no way to look at a pull request. Gitplace hosts
// the same pane at full width, for any project Bridge knows about, and needs
// no session: every GitHub read is keyed on the workspace alone.

export const GITPLACE_REPO_KEY = "bridge.gitplace.workspace";

export type GitplaceRepo = { workspace: Workspace; label: string };

/// One entry per project: its task worktrees share a remote, so listing each
/// of them would show the same repository several times. The newest worktree
/// stands for the project. Workspaces with no folder have nothing to read.
export function gitplaceRepos(workspaces: Workspace[], projects: Project[]): GitplaceRepo[] {
  const byKey = new Map<string, Workspace>();
  for (const workspace of workspaces) {
    if (!workspace.path) continue;
    const key = workspace.projectId ?? workspace.id;
    const held = byKey.get(key);
    if (!held || Date.parse(workspace.createdAt) > Date.parse(held.createdAt)) byKey.set(key, workspace);
  }
  return [...byKey.values()]
    .map(workspace => ({ workspace, label: projects.find(project => project.id === workspace.projectId)?.name ?? workspace.title }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

function readRemembered(): string | null {
  try { return window.localStorage.getItem(GITPLACE_REPO_KEY); } catch { return null; }
}

function remember(workspaceId: string) {
  try { window.localStorage.setItem(GITPLACE_REPO_KEY, workspaceId); } catch { /* private mode: not remembered */ }
}

export function GitplaceScreen({ workspaces, projects, onJumpToFile, onAddProject }: {
  workspaces: Workspace[];
  projects: Project[];
  /** A review comment's file lives in a worktree, so opening it needs a chat
   *  in that workspace; the host finds or starts one. */
  onJumpToFile: (workspaceId: string, path: string, line: number | undefined, headBranch: string) => void;
  onAddProject: () => void;
}) {
  const repos = useMemo(() => gitplaceRepos(workspaces, projects), [workspaces, projects]);
  const [chosen, setChosen] = useState<string | null>(readRemembered);
  const attribution = useAttributionSettings();
  const current = repos.find(repo => repo.workspace.id === chosen)
    ?? repos.find(repo => repo.workspace.projectId && repo.workspace.projectId === workspaces.find(workspace => workspace.id === chosen)?.projectId)
    ?? repos[0];
  const menu = useMenuPanel<HTMLButtonElement>({ width: 280, height: Math.min(320, 12 + repos.length * 32) });

  if (!current) {
    return <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
      <FolderGit2 size={18} className="text-muted-foreground" aria-hidden="true" />
      <h2 className="text-[15px] font-medium text-foreground">Add a project with a GitHub remote</h2>
      <p className="max-w-sm text-[13px] text-muted-foreground">Gitplace shows pull requests and issues for the projects Bridge knows about. None has a folder yet.</p>
      <button type="button" onClick={onAddProject} className="mt-2 min-h-8 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90">Add a project</button>
    </div>;
  }

  const choose = (workspaceId: string) => {
    setChosen(workspaceId);
    remember(workspaceId);
    menu.close();
  };

  return <div data-gitplace className="flex h-full min-h-0 flex-col">
    <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
      <button
        ref={menu.triggerRef}
        type="button"
        onClick={menu.toggle}
        aria-haspopup="menu"
        aria-expanded={menu.open}
        aria-label={`Repository: ${current.label}`}
        className="flex h-8 min-w-0 items-center gap-1.5 rounded-md px-2 text-[13px] text-foreground transition-colors hover:bg-accent"
      >
        <FolderGit2 size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="truncate font-medium">{current.label}</span>
        <ChevronDown size={14} className={cn("shrink-0 text-muted-foreground transition-transform", menu.open && "rotate-180")} aria-hidden="true" />
      </button>
      <div className="ml-auto flex shrink-0 items-center gap-2" title="When on, models never add Co-authored-by or harness mentions to commits or PR text">
        <span className="hidden text-[12px] text-muted-foreground sm:inline">Hide AI attribution</span>
        <Switch
          label="Hide AI attribution"
          checked={attribution.hide}
          onChange={attribution.setHide}
          disabled={!attribution.loaded || attribution.saving}
        />
        {attribution.error && <span role="alert" className="max-w-44 truncate text-[12px] text-destructive" title={attribution.error}>Not saved</span>}
      </div>
    </div>
    <MenuPanel controller={menu} label="Repositories">
      {repos.map(repo => <MenuItem
        key={repo.workspace.id}
        role="menuitemradio"
        checked={repo.workspace.id === current.workspace.id}
        label={repo.label}
        trailing={repo.workspace.id === current.workspace.id ? <Check size={13} aria-hidden="true" /> : undefined}
        onClick={() => choose(repo.workspace.id)}
      />)}
    </MenuPanel>
    <div className="min-h-0 flex-1">
      {/* Keyed on the workspace: a switch must not carry one repository's
          selection, filters or detail into another's. */}
      <GitHubPane
        key={current.workspace.id}
        layout="page"
        workspaceId={current.workspace.id}
        workspaceBranch={current.workspace.branch ?? null}
        onJumpToFile={(path, line, headBranch) => onJumpToFile(current.workspace.id, path, line, headBranch)}
      />
    </div>
  </div>;
}
