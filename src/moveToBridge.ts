// The logic behind "Move to Bridge": what a found tool holds, and how far a
// background import has got. The import itself runs in Bridge's own process
// (bridge-core `import_job`), which also adds up what came over. Pure, so it
// is tested without a DOM.
import type { DetectedImportSource, ImportJob } from "./protocol/generated/protocol";

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

/** Whole percent of the files read; 0 until the job has counted them. */
export function importPercent(job: Pick<ImportJob, "processed" | "total">): number {
  if (job.total <= 0) return 0;
  return Math.min(100, Math.floor((job.processed / job.total) * 100));
}

/**
 * The job to show: a live event and a status read can arrive in either
 * order, so the older report of the same job never replaces a newer one.
 */
export function newerJob(current: ImportJob | null, next: ImportJob): ImportJob {
  if (!current || current.jobId !== next.jobId) return next;
  return next.updatedAt >= current.updatedAt ? next : current;
}

/** When a finished import ended, for the summary heading. */
export function finishedLabel(job: ImportJob): string | undefined {
  if (!job.finishedAt) return undefined;
  return `Finished ${new Date(job.finishedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}`;
}

/** What Bridge says when an import ends: in a toast, and as a system notification. */
export function importNotice(job: ImportJob, toolLabel = "Claude Code"): { title: string; body: string; failed: boolean } {
  if (job.status === "failed") {
    return { title: "Import stopped", body: job.errors?.[0] ?? `The import from ${toolLabel} could not finish.`, failed: true };
  }
  const parts = [`${job.imported.toLocaleString()} ${job.imported === 1 ? "item" : "items"} came over from ${toolLabel}`];
  if (job.alreadyInBridge > 0) parts.push(`${job.alreadyInBridge.toLocaleString()} already in Bridge`);
  if (job.failed > 0) parts.push(`${job.failed.toLocaleString()} could not be imported`);
  return { title: "Import finished", body: `${parts.join(", ")}.`, failed: false };
}
