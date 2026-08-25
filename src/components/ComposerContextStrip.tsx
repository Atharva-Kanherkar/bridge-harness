import { Cloud, FolderGit2, GitBranch, GitFork, Laptop, Terminal } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuItem, MenuPanel, useMenuPanel } from "@/components/ui/menu-panel";
import type { Workspace } from "../types";

export type ComposerContextStripProps = {
  workspaces: Workspace[];
  workspace: Workspace | null;
  worktree: boolean;
  /** Repo menu and worktree toggle lock after the first user turn. */
  locked: boolean;
  branches: string[];
  branchBusy?: boolean;
  branchError?: string | null;
  onSelectWorkspace: (workspaceId: string) => void;
  onRequestBranches: () => void;
  onSelectBranch: (branch: string) => void;
  onToggleWorktree: () => void;
};

const HOSTS: { id: "local" | "cloud" | "ssh"; label: string; icon: LucideIcon; hint: string; disabled: boolean }[] = [
  { id: "local", label: "This Mac", icon: Laptop, hint: "Run the agent on this machine", disabled: false },
  { id: "cloud", label: "Cloud", icon: Cloud, hint: "Not wired up yet", disabled: true },
  { id: "ssh", label: "SSH", icon: Terminal, hint: "Not wired up yet", disabled: true },
];

function Chip({
  icon: Icon,
  label,
  pressed,
  expanded,
  disabled,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  pressed?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}) {
  const className = cn(
    "inline-flex h-7 max-w-[16rem] items-center gap-1.5 rounded-md px-2 text-[12.5px] tracking-[-0.01em]",
    pressed || expanded ? "bg-accent text-foreground" : "text-muted-foreground",
    onClick && !disabled && "transition-colors hover:bg-accent hover:text-foreground",
    disabled && "opacity-70",
  );
  if (!onClick) {
    return (
      <span className={className}>
        <Icon size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">{label}</span>
      </span>
    );
  }
  return (
    <button type="button" aria-pressed={pressed} aria-expanded={expanded} disabled={disabled} onClick={onClick} className={className}>
      <Icon size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
      <span className="min-w-0 truncate">{label}</span>
    </button>
  );
}

export function ComposerContextStrip({
  workspaces,
  workspace,
  worktree,
  locked,
  branches,
  branchBusy = false,
  branchError,
  onSelectWorkspace,
  onRequestBranches,
  onSelectBranch,
  onToggleWorktree,
}: ComposerContextStripProps) {
  const repoMenu = useMenuPanel<HTMLButtonElement>({ width: 220, height: 220 });
  const branchMenu = useMenuPanel<HTMLButtonElement>({ width: 240, height: 260 });
  const hostMenu = useMenuPanel<HTMLButtonElement>({ width: 240, height: 180 });
  const worktreeAvailable = !!workspace?.projectId;
  const branchAvailable = worktreeAvailable && !worktree;

  return (
    <div className="mb-2 flex flex-nowrap items-center justify-center gap-0.5 overflow-x-auto px-3" aria-label="Chat context">
      <button
        type="button"
        ref={repoMenu.triggerRef}
        aria-haspopup="menu"
        aria-expanded={repoMenu.open}
        disabled={locked || workspaces.length === 0}
        onClick={repoMenu.toggle}
        className={cn(
          "inline-flex h-7 max-w-[16rem] items-center gap-1.5 rounded-md px-2 text-[12.5px] tracking-[-0.01em] text-muted-foreground transition-colors",
          !locked && workspaces.length > 0 && "hover:bg-accent hover:text-foreground",
          repoMenu.open && "bg-accent text-foreground",
        )}
      >
        <FolderGit2 size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">{workspace?.title ?? "No project"}</span>
      </button>
      <MenuPanel controller={repoMenu} label="Repository">
        {workspaces.map(item => (
          <MenuItem
            key={item.id}
            role="menuitemradio"
            checked={item.id === workspace?.id}
            label={item.title}
            leading={<FolderGit2 size={13} aria-hidden="true" />}
            onClick={() => { onSelectWorkspace(item.id); repoMenu.close(); }}
          />
        ))}
      </MenuPanel>

      <button
        type="button"
        ref={branchMenu.triggerRef}
        aria-haspopup="menu"
        aria-expanded={branchMenu.open}
        disabled={locked || !branchAvailable}
        onClick={() => {
          if (!branchBusy) onRequestBranches();
          branchMenu.toggle();
        }}
        className={cn(
          "inline-flex h-7 max-w-[16rem] items-center gap-1.5 rounded-md px-2 text-[12.5px] tracking-[-0.01em] text-muted-foreground transition-colors",
          !locked && branchAvailable && "hover:bg-accent hover:text-foreground",
          branchMenu.open && "bg-accent text-foreground",
          (locked || !branchAvailable || branchBusy) && "opacity-70",
        )}
      >
        <GitBranch size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">{workspace?.branch || "No branch"}</span>
      </button>
      <MenuPanel controller={branchMenu} label="Branch">
        {branchBusy && <p role="status" aria-live="polite" className="px-3 py-2 text-[12px] text-muted-foreground">Loading branches…</p>}
        {!branchBusy && branchError && <p role="status" aria-live="polite" className="px-3 py-2 text-[12px] text-destructive">{branchError}</p>}
        {!branchBusy && !branchError && branches.map(branch => (
          <MenuItem
            key={branch}
            role="menuitemradio"
            checked={branch === workspace?.branch}
            label={branch}
            leading={<GitBranch size={13} aria-hidden="true" />}
            onClick={() => {
              if (branch !== workspace?.branch) onSelectBranch(branch);
              branchMenu.close();
            }}
          />
        ))}
        {!branchBusy && !branchError && branches.length === 0 && (
          <p role="status" className="px-3 py-2 text-[12px] text-muted-foreground">No local branches</p>
        )}
      </MenuPanel>

      <Chip
        icon={GitFork}
        label={worktree ? "Isolated worktree" : "On branch"}
        pressed={worktree}
        disabled={locked || !worktreeAvailable}
        onClick={locked || !worktreeAvailable ? undefined : onToggleWorktree}
      />

      <button
        type="button"
        ref={hostMenu.triggerRef}
        aria-haspopup="menu"
        aria-expanded={hostMenu.open}
        onClick={hostMenu.toggle}
        className={cn(
          "inline-flex h-7 max-w-[16rem] items-center gap-1.5 rounded-md px-2 text-[12.5px] tracking-[-0.01em] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
          hostMenu.open && "bg-accent text-foreground",
        )}
      >
        <Laptop size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">This Mac</span>
      </button>
      <MenuPanel controller={hostMenu} label="Agent host">
        {HOSTS.map(item => {
          const Icon = item.icon;
          return (
            <MenuItem
              key={item.id}
              label={item.label}
              disabled={item.disabled}
              checked={item.id === "local"}
              role="menuitemradio"
              leading={<Icon size={13} aria-hidden="true" />}
              trailing={<span className="text-[11px] text-muted-foreground">{item.hint}</span>}
              onClick={() => hostMenu.close()}
            />
          );
        })}
      </MenuPanel>
    </div>
  );
}
