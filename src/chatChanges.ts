import { useEffect, useState } from "react";
import { bridgeApi } from "./api";
import { startSerialPoll } from "./polling";
import type { SessionChangesResult, Workspace } from "./types";

/**
 * What the header's Review badge counts.
 *
 * A chat's own changes when Bridge has measured them: the files its turns
 * changed since its first turn. Counting the folder's whole `git status`
 * instead put weeks of other work under every chat in a shared checkout.
 * The folder's count is the fallback only when the chat cannot be measured
 * (no turn yet, an older daemon, a failed snapshot), and it is labelled as
 * the folder's.
 */
export interface ReviewBadge {
  scope: "chat" | "folder";
  files: number;
  additions: number;
  deletions: number;
}

export function reviewBadge(
  chat: SessionChangesResult | undefined,
  chatLoaded: boolean,
  workspace: Pick<Workspace, "dirtyFiles" | "additions" | "deletions">,
): ReviewBadge | null {
  if (chat?.available && chat.changes) {
    const files = chat.changes.totalFiles ?? chat.changes.files.length;
    return files > 0 ? { scope: "chat", files, additions: chat.additions, deletions: chat.deletions } : null;
  }
  // Not measured yet: say nothing for a moment rather than flash the
  // folder's number and then shrink to the chat's.
  if (!chatLoaded) return null;
  return workspace.dirtyFiles > 0
    ? { scope: "folder", files: workspace.dirtyFiles, additions: workspace.additions, deletions: workspace.deletions }
    : null;
}

const sameStats = (a: SessionChangesResult, b: SessionChangesResult) =>
  a.available === b.available && a.unavailableReason === b.unavailableReason && a.turns === b.turns
  && a.additions === b.additions && a.deletions === b.deletions
  && (a.changes?.totalFiles ?? a.changes?.files.length) === (b.changes?.totalFiles ?? b.changes?.files.length)
  && (a.changes?.files ?? []).every((file, index) => {
    const other = b.changes?.files[index];
    return !!other && other.path === file.path && other.additions === file.additions && other.deletions === file.deletions;
  });

/**
 * Poll the open chat's own change counts (no patches) on the same clock as
 * the workspace stats. `loaded` turns true once this chat has an answer,
 * including a failed one, so the badge can fall back instead of waiting.
 */
export function useChatChanges(sessionId: string | undefined, enabled: boolean, intervalMs = 5000) {
  const [state, setState] = useState<{ sessionId: string; result?: SessionChangesResult; failed?: boolean }>();
  useEffect(() => {
    if (!sessionId || !enabled) return;
    let active = true;
    const stop = startSerialPoll(async () => {
      try {
        const result = await bridgeApi.sessionChanges(sessionId);
        if (active) setState(current => current?.sessionId === sessionId && current.result && sameStats(current.result, result) ? current : { sessionId, result });
      } catch {
        if (active) setState(current => current?.sessionId === sessionId && current.failed ? current : { sessionId, failed: true });
      }
    }, intervalMs);
    return () => { active = false; stop(); };
  }, [sessionId, enabled, intervalMs]);
  const current = state?.sessionId === sessionId ? state : undefined;
  const result = current?.result;
  const files = result?.changes?.totalFiles ?? result?.changes?.files.length ?? 0;
  return {
    result,
    loaded: !sessionId || !enabled || !!current,
    /** Changes when the chat's counts drift: the Changes pane's cue to re-read. */
    signal: result ? `${result.turns}:${files}:${result.additions}:${result.deletions}` : "",
  };
}
