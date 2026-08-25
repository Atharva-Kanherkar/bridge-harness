import type { Workspace } from "./types";

export const LAST_WORKSPACE_KEY = "bridge.chat.lastWorkspaceId";

export function writeLastWorkspaceId(id: string): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(LAST_WORKSPACE_KEY, id);
}

export function readLastWorkspaceId(): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(LAST_WORKSPACE_KEY);
}

/** First match wins: the chat already on screen, then the last repo the user
 * was in, then the only workspace. Stale ids fall through rather than creating
 * a session in a workspace that no longer exists. */
export function resolveNewChatWorkspaceId(args: {
  activeWorkspaceId?: string | null;
  lastWorkspaceId?: string | null;
  workspaces: Pick<Workspace, "id">[];
}): string | null {
  const ids = new Set(args.workspaces.map(workspace => workspace.id));
  if (args.activeWorkspaceId && ids.has(args.activeWorkspaceId)) return args.activeWorkspaceId;
  if (args.lastWorkspaceId && ids.has(args.lastWorkspaceId)) return args.lastWorkspaceId;
  if (args.workspaces.length === 1) return args.workspaces[0].id;
  return null;
}
