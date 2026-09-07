import { ChevronDown, Cloud, FolderGit2, GitBranch, GitFork, Laptop, Plus, Terminal } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuItem, MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";
import type { Workspace } from "../types";

export type ComposerContextStripProps = {
  workspaces: Workspace[];
  workspace: Workspace | null;
  worktree: boolean;
  /** Existing chats keep their checkout; menus still explain the restriction. */
  locked: boolean;
  lockReason?: string;
  onNewChat?: () => void;
  branches: string[];
  /** Git's HEAD name when it differs from the last stored `workspace.branch`. */
  currentBranch?: string | null;
  branchBusy?: boolean;
  branchError?: string | null;
  onSelectWorkspace: (workspaceId: string) => void;
  onRequestBranches: () => void;
  onSelectBranch: (branch: string) => void;
  onToggleWorktree: () => void;
};

const HOSTS: { id: "local" | "cloud" | "ssh"; label: string; icon: LucideIcon; disabled: boolean }[] = [
  { id: "local", label: "This Mac", icon: Laptop, disabled: false },
  { id: "cloud", label: "Cloud", icon: Cloud, disabled: true },
  { id: "ssh", label: "SSH", icon: Terminal, disabled: true },
];

function ContextHelp({ text, onNewChat }: { text: string; onNewChat?: () => void }) {
  return <>
    <p className="px-3 py-2 text-caption text-muted-foreground">{text}</p>
    {onNewChat && <>
      <MenuSeparator />
      <MenuItem label="New chat with different settings…" leading={<Plus size={13} aria-hidden="true" />} onClick={onNewChat} />
    </>}
  </>;
}

export function ComposerContextStrip({
  workspaces,
  workspace,
  worktree,
  locked,
  lockReason = "Project and work mode are fixed after the first message. Choose different settings in a new chat.",
  onNewChat,
  branches,
  currentBranch,
  branchBusy = false,
  branchError,
  onSelectWorkspace,
  onRequestBranches,
  onSelectBranch,
  onToggleWorktree,
}: ComposerContextStripProps) {
  const repoMenu = useMenuPanel<HTMLButtonElement>({ width: 280, height: 260 });
  const branchMenu = useMenuPanel<HTMLButtonElement>({ width: 280, height: 260 });
  const hostMenu = useMenuPanel<HTMLButtonElement>({ width: 280, height: 220 });
  const worktreeMenu = useMenuPanel<HTMLButtonElement>({ width: 280, height: 260 });
  const worktreeAvailable = !!workspace?.projectId;
  const branchAvailable = worktreeAvailable && !worktree;
  const displayedBranch = currentBranch || workspace?.branch || "No branch";

  return (
    <div className="@container/composer-context flex min-h-7 items-center gap-0.5 text-muted-foreground" aria-label="Chat context">
      <button
        type="button"
        ref={repoMenu.triggerRef}
        aria-haspopup="menu"
        aria-expanded={repoMenu.open}
        title={workspace?.title ?? "No project"}
        onClick={repoMenu.toggle}
        className={cn(
          "inline-flex h-7 min-w-0 max-w-[16rem] flex-1 items-center gap-1.5 rounded-md px-2 text-[12px] tracking-[-0.01em] text-muted-foreground transition-colors",
          "hover:bg-accent hover:text-foreground",
          repoMenu.open && "bg-accent text-foreground",
        )}
      >
        <FolderGit2 size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">{workspace?.title ?? "No project"}</span>
        <ChevronDown size={10} className="shrink-0 opacity-60" aria-hidden="true" />
      </button>
      <MenuPanel controller={repoMenu} label="Repository">
        {(locked ? workspace ? [workspace] : [] : workspaces).map(item => (
          <MenuItem
            key={item.id}
            role="menuitemradio"
            checked={item.id === workspace?.id}
            disabled={locked}
            label={item.title}
            leading={<FolderGit2 size={13} aria-hidden="true" />}
            onClick={() => { onSelectWorkspace(item.id); repoMenu.close(); }}
          />
        ))}
        {locked && <ContextHelp text={lockReason} onNewChat={onNewChat ? () => { repoMenu.close(); onNewChat(); } : undefined} />}
        {!locked && workspaces.length === 0 && <ContextHelp text="Add a project to choose a repository for this chat." />}
      </MenuPanel>

      <button
        type="button"
        ref={branchMenu.triggerRef}
        aria-haspopup="menu"
        aria-expanded={branchMenu.open}
        title={displayedBranch}
        onClick={() => {
          if (!locked && branchAvailable && !branchBusy) onRequestBranches();
          branchMenu.toggle();
        }}
        className={cn(
          "inline-flex h-7 min-w-0 max-w-[16rem] flex-1 items-center gap-1.5 rounded-md px-2 text-[12px] tracking-[-0.01em] text-muted-foreground transition-colors",
          "hover:bg-accent hover:text-foreground",
          branchMenu.open && "bg-accent text-foreground",
          branchBusy && "opacity-70",
        )}
      >
        <GitBranch size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">{displayedBranch}</span>
        <ChevronDown size={10} className="shrink-0 opacity-60" aria-hidden="true" />
      </button>
      <MenuPanel controller={branchMenu} label="Branch">
        {locked || !branchAvailable ? <>
          <MenuItem label={displayedBranch} role="menuitemradio" checked disabled onClick={() => {}} leading={<GitBranch size={13} aria-hidden="true" />} />
          <ContextHelp
            text={locked ? "This chat keeps its current checkout. Branch switching requires the chats and terminal using that checkout to be stopped." : worktree ? "This chat uses its own isolated worktree. The project checkout is unchanged." : "Connect a Git repository to choose a branch."}
            onNewChat={locked && onNewChat ? () => { branchMenu.close(); onNewChat(); } : undefined}
          />
        </> : <>
          {branchBusy && <p role="status" aria-live="polite" className="px-3 py-2 text-[12px] text-muted-foreground">Loading branches…</p>}
          {!branchBusy && branchError && <p role="status" aria-live="polite" className="px-3 py-2 text-[12px] text-destructive">{branchError}</p>}
          {!branchBusy && !branchError && branches.map(branch => (
            <MenuItem
              key={branch}
              role="menuitemradio"
              checked={branch === displayedBranch}
              label={branch}
              leading={<GitBranch size={13} aria-hidden="true" />}
              onClick={() => {
                if (branch !== displayedBranch) onSelectBranch(branch);
                branchMenu.close();
              }}
            />
          ))}
          {!branchBusy && !branchError && branches.length === 0 && (
            <p role="status" className="px-3 py-2 text-[12px] text-muted-foreground">No local branches</p>
          )}
        </>}
      </MenuPanel>

      <button
        type="button"
        ref={worktreeMenu.triggerRef}
        aria-label={worktree ? "Work mode: Isolated worktree" : "Work mode: Work on branch"}
        aria-haspopup="menu"
        aria-expanded={worktreeMenu.open}
        title={worktree ? "Isolated worktree" : "Work on branch"}
        onClick={worktreeMenu.toggle}
        className={cn(
          "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
          worktreeMenu.open && "bg-accent text-foreground",
        )}
      >
        <GitFork size={13} strokeWidth={1.7} aria-hidden="true" />
        <span className="hidden @xl/composer-context:inline">{worktree ? "Isolated worktree" : "Work on branch"}</span>
        <ChevronDown size={10} className="shrink-0 opacity-60" aria-hidden="true" />
      </button>
      <MenuPanel controller={worktreeMenu} label="Work mode">
        {[false, true].map(isolated => <MenuItem
          key={String(isolated)}
          label={isolated ? "Isolated worktree" : "Work on branch"}
          role="menuitemradio"
          checked={worktree === isolated}
          disabled={locked || !worktreeAvailable}
          leading={<GitFork size={13} aria-hidden="true" />}
          onClick={() => {
            if (isolated !== worktree) onToggleWorktree();
            worktreeMenu.close();
          }}
        />)}
        <ContextHelp
          text={locked ? lockReason : !worktreeAvailable ? "Connect a Git repository to create an isolated worktree." : "Use the project checkout, or give this chat an isolated copy in its own worktree."}
          onNewChat={locked && onNewChat ? () => { worktreeMenu.close(); onNewChat(); } : undefined}
        />
      </MenuPanel>

      <button
        type="button"
        ref={hostMenu.triggerRef}
        aria-label="Agent host: This Mac"
        aria-haspopup="menu"
        aria-expanded={hostMenu.open}
        title="This Mac"
        onClick={hostMenu.toggle}
        className={cn(
          "ml-auto inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
          hostMenu.open && "bg-accent text-foreground",
        )}
      >
        <Laptop size={13} strokeWidth={1.7} aria-hidden="true" />
        <span className="hidden @xl/composer-context:inline">This Mac</span>
        <ChevronDown size={10} className="shrink-0 opacity-60" aria-hidden="true" />
      </button>
      <MenuPanel controller={hostMenu} label="Agent host">
        {HOSTS.map(item => {
          const Icon = item.icon;
          return <MenuItem
            key={item.id}
            label={item.label}
            disabled={item.disabled}
            checked={item.id === "local"}
            role="menuitemradio"
            leading={<Icon size={13} aria-hidden="true" />}
            trailing={item.disabled ? <span className="text-caption text-muted-foreground">Unavailable</span> : undefined}
            onClick={() => hostMenu.close()}
          />;
        })}
        <ContextHelp text="Agents run on this Mac. Cloud and SSH hosts are not available yet." />
      </MenuPanel>
    </div>
  );
}
