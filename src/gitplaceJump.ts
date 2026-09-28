import type { AppView } from "./navigationHistory";

/** A review-comment location clicked in Gitplace, waiting for a chat in its
 *  workspace to be on screen: the file lives in that worktree, and only a chat
 *  has the Code pane to open it in. */
export type GitplaceJump = { workspaceId: string; path: string; line: number | undefined; headBranch: string };

/// What to do with a pending jump given what is on screen now.
///
/// - `jump`: a chat in the jump's workspace is open, so open the file.
/// - `wait`: an unstarted draft in that workspace is open. The chat does not
///   exist until its first message is sent, and the jump must survive that.
/// - `drop`: the reader went somewhere else (another chat, another draft,
///   another view), so the jump is stale and must not fire later by surprise.
export function gitplaceJumpStep(
  jump: GitplaceJump,
  now: { view: AppView; sessionWorkspaceId?: string | null; draftWorkspaceId?: string | null },
): "jump" | "wait" | "drop" {
  if (now.view !== "workspace") return "drop";
  if (now.sessionWorkspaceId) return now.sessionWorkspaceId === jump.workspaceId ? "jump" : "drop";
  return now.draftWorkspaceId === jump.workspaceId ? "wait" : "drop";
}
