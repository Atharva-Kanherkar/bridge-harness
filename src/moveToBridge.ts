// The logic behind "Move to Bridge": grouping what a tool holds into projects
// for review, slicing an import into batches the daemon can carry, and adding
// up what came over and what did not. Pure, so it is tested without a DOM.

import type {
  DetectedImportSource,
  ExternalImportArtifact,
  ExternalImportCandidate,
  ExternalImportCommit,
} from "./protocol/generated/protocol";

/** Artifacts previewed and committed per round trip. */
export const IMPORT_BATCH_SIZE = 25;

export type ChatRow = { artifactId: string; title: string; startedAt: string | null; bytes: number };
export type ProjectGroup = { key: string; label: string; chats: ChatRow[]; latest: string; background: boolean };

/** Chats that ran in the OS temp folder are SDK probes and background runs. */
export function isBackgroundFolder(path: string | null | undefined): boolean {
  return !!path && /^\/(private\/)?(var\/folders|tmp)\//.test(path + "/");
}

export const BACKGROUND_GROUP = "background";

/** "312 chats in 14 projects · 3 memories" for a found tool, or why it is empty. */
export function sourceSummary(source: DetectedImportSource): string {
  if (!source.installed) return "Not found on this Mac";
  const { chats, projects, workspaces, memories, setup } = source.counts;
  const background = source.counts.background ?? 0;
  const parts: string[] = [];
  if (chats > 0) parts.push(`${chats.toLocaleString()} ${chats === 1 ? "chat" : "chats"}${projects > 0 && workspaces === 0 ? ` in ${projects.toLocaleString()} ${projects === 1 ? "project" : "projects"}` : ""}`);
  if (workspaces > 0) parts.push(`${workspaces.toLocaleString()} ${workspaces === 1 ? "workspace" : "workspaces"} in ${projects.toLocaleString()} ${projects === 1 ? "repo" : "repos"}`);
  if (memories > 0) parts.push(`${memories.toLocaleString()} ${memories === 1 ? "memory" : "memories"}`);
  if (setup > 0) parts.push(`${setup.toLocaleString()} setup ${setup === 1 ? "item" : "items"}`);
  if (background > 0) parts.push(`${background.toLocaleString()} background ${background === 1 ? "run" : "runs"} left out`);
  return parts.length > 0 ? parts.join(" · ") : "Found, but nothing to bring over yet";
}

/** A chat's folder as a short label: the last two path segments. */
export function folderLabel(path: string | null | undefined): string {
  if (!path) return "Folder unknown";
  const parts = path.split("/").filter(Boolean);
  return parts.length <= 2 ? `/${parts.join("/")}` : `…/${parts.slice(-2).join("/")}`;
}

/** Chats grouped by the folder they ran in, most recent project first. */
export function groupChats(artifacts: ExternalImportArtifact[]): ProjectGroup[] {
  const groups = new Map<string, ProjectGroup>();
  for (const artifact of artifacts) {
    if (artifact.kind !== "conversation") continue;
    const background = isBackgroundFolder(artifact.projectPath);
    const key = background ? BACKGROUND_GROUP : artifact.projectPath ?? "";
    const startedAt = artifact.startedAt ?? artifact.modifiedAt ?? null;
    const group = groups.get(key) ?? { key, label: background ? "Background runs" : folderLabel(artifact.projectPath), chats: [], latest: "", background };
    group.chats.push({
      artifactId: artifact.artifactId,
      title: artifact.title?.trim() || artifact.sourceLabel.split("/").pop() || "Untitled chat",
      startedAt,
      bytes: artifact.estimatedBytes,
    });
    if (startedAt && startedAt > group.latest) group.latest = startedAt;
    groups.set(key, group);
  }
  for (const group of groups.values()) {
    group.chats.sort((a, b) => (b.startedAt ?? "").localeCompare(a.startedAt ?? ""));
  }
  // Background runs go last whatever their dates.
  return [...groups.values()].sort((a, b) => Number(a.background) - Number(b.background) || b.latest.localeCompare(a.latest));
}

export function chunk<T>(items: T[], size = IMPORT_BATCH_SIZE): T[][] {
  const out: T[][] = [];
  for (let index = 0; index < items.length; index += size) out.push(items.slice(index, index + size));
  return out;
}

/** Which previewed candidates a batch commits: everything importable that the person kept. */
export function importableIds(candidates: ExternalImportCandidate[]): string[] {
  return candidates.filter(candidate => candidate.kind !== "unsupported").map(candidate => candidate.candidateId);
}

export type ImportReport = {
  imported: number;
  alreadyInBridge: number;
  changed: number;
  failed: number;
  /** "3 attachments: image too large…", merged by message text. */
  notCarried: { text: string; count: number }[];
  errors: string[];
};

export function emptyReport(): ImportReport {
  return { imported: 0, alreadyInBridge: 0, changed: 0, failed: 0, notCarried: [], errors: [] };
}

const NUMBER_PREFIX = /^(\d+)\s+/;

/** Fold one batch's preview and commit into the running report. */
export function addBatch(report: ImportReport, candidates: ExternalImportCandidate[], commit: ExternalImportCommit | null, error?: string): ImportReport {
  const next: ImportReport = { ...report, notCarried: [...report.notCarried], errors: [...report.errors] };
  if (commit) {
    next.imported += commit.imported;
    next.alreadyInBridge += commit.skipped;
    next.changed += commit.changed;
    next.failed += commit.rejected + commit.conflicted;
    for (const diagnostic of commit.diagnostics) if (diagnostic.severity === "error") next.errors.push(diagnostic.message);
  }
  if (error) next.errors.push(error);
  for (const candidate of candidates) {
    if (candidate.kind === "unsupported") {
      next.failed += 1;
      const reason = candidate.diagnostics[0]?.message ?? "could not be read";
      add(next, `A chat could not be read: ${reason}`, 1);
      continue;
    }
    for (const diagnostic of candidate.diagnostics) {
      if (!diagnostic.code.startsWith("excluded_")) continue;
      const match = NUMBER_PREFIX.exec(diagnostic.message);
      add(next, match ? diagnostic.message.slice(match[0].length) : diagnostic.message, match ? Number(match[1]) : 1);
    }
  }
  return next;
}

function add(report: ImportReport, text: string, count: number) {
  const existing = report.notCarried.find(item => item.text === text);
  if (existing) existing.count += count;
  else report.notCarried.push({ text, count });
}
