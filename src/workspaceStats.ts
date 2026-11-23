import type { BridgeState, Workspace } from "./types";

/** `state` with one workspace's stored row swapped in, or `state` itself when
 *  nothing the app shows moved, so a quiet poll re-renders nothing. */
export function withWorkspace(state: BridgeState, workspace: Workspace): BridgeState {
  const index = state.workspaces.findIndex(candidate => candidate.id === workspace.id);
  if (index < 0) return state;
  const current = state.workspaces[index];
  const same = (Object.keys(workspace) as Array<keyof Workspace>).every(key => current[key] === workspace[key]);
  if (same) return state;
  const workspaces = state.workspaces.slice();
  workspaces[index] = workspace;
  return { ...state, workspaces };
}
