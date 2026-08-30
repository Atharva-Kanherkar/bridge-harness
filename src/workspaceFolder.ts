import type { Workspace } from "./types";

export function selectedFolder(value: string | string[] | null): string | null {
  return typeof value === "string" ? value : null;
}

export function workspaceTitleFromFolder(path: string): string {
  return path.split("/").filter(Boolean).at(-1) ?? "Folder";
}

export function workspaceForFolder(workspaces: Workspace[], path: string): Workspace | undefined {
  return workspaces.find(workspace => workspace.path === path);
}
