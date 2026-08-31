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

// Mirrors the three URL shapes `validate_github_url` (project_onboarding.rs)
// actually accepts — github.com only, https/http, SCP-style git@, or ssh://.
// Anything else would just fail the clone, so it isn't a clone target here.
const GITHUB_URL = /^(?:https?:\/\/github\.com\/|git@github\.com:|ssh:\/\/git@github\.com\/)[^/\s]+\/[^/\s]+\/?$/i;
const OWNER_REPO_SHORTHAND = /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/;
const FILE_EXTENSION = /\.(tsx?|jsx?|py|rs|go|rb|java|c|cpp|h|hpp|md|mdx|json|ya?ml|toml|css|scss|html?|txt|sh|sql)$/i;

// A bare GitHub URL, or an "owner/repo" shorthand that isn't shaped like a
// file path someone pasted (e.g. "src/App.tsx"), sent as the entire message
// with nothing else — that's someone naming a project to open, not chatting.
export function repoCloneTarget(text: string): string | undefined {
  const trimmed = text.trim();
  if (!trimmed || /\s/.test(trimmed)) return undefined;
  if (GITHUB_URL.test(trimmed)) return trimmed;
  if (OWNER_REPO_SHORTHAND.test(trimmed) && !FILE_EXTENSION.test(trimmed)) return `https://github.com/${trimmed}`;
  return undefined;
}
