import type { GithubIssuesResult, GithubRepositoryResult, GithubStatusResult } from "./protocol/generated/protocol";
import type { PullRequestListItem } from "./githubSurface";

// The last-known GitHub surface read for one workspace, remembered across
// mounts so a reopened pane paints content immediately and revalidates in
// the background, instead of sitting on a skeleton every time the surface is
// opened. The pane remains the source of truth: a remembered snapshot is only
// a first paint, and every open still refetches.

export type GithubSurfaceSnapshot = {
  status: GithubStatusResult;
  prs?: PullRequestListItem[];
  issues?: GithubIssuesResult["issues"];
  repositoryOverview?: GithubRepositoryResult;
};

const REMEMBERED = 8;
const snapshots = new Map<string, GithubSurfaceSnapshot>();

export function surfaceSnapshot(workspaceId: string): GithubSurfaceSnapshot | undefined {
  return snapshots.get(workspaceId);
}

/** Merge `snapshot` over whatever was already remembered for the workspace:
 *  a status read that succeeded while a list read failed must not erase the
 *  last good list. Re-inserted so the map stays least-recently-used ordered. */
export function rememberSurface(workspaceId: string, snapshot: GithubSurfaceSnapshot) {
  const previous = snapshots.get(workspaceId);
  const repository = snapshot.status.repository;
  const held = previous?.status.repository;
  const sameRepository = snapshot.status.availability.status === "available"
    && previous?.status.availability.status === "available"
    && repository && held
    && repository.host === held.host && repository.owner === held.owner && repository.name === held.name;
  const merged = { ...(sameRepository ? previous : undefined), ...snapshot };
  snapshots.delete(workspaceId);
  snapshots.set(workspaceId, merged);
  while (snapshots.size > REMEMBERED) {
    const oldest = snapshots.keys().next().value;
    if (oldest === undefined) break;
    snapshots.delete(oldest);
  }
  return merged;
}

/** Test seam: the cache outlives components by design, so a test that seeds
 *  it must be able to empty it again. */
export function forgetSurfaces() {
  snapshots.clear();
}
