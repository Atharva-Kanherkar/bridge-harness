// The logic behind "Move to Bridge": what a found tool holds, and how far a
// background import has got. The import itself runs in Bridge's own process
// (bridge-core `import_job`), which also adds up what came over. Pure, so it
// is tested without a DOM.
import type { DetectedImportSource, ImportJob } from "./protocol/generated/protocol";

/**
 * "312 chats in 14 projects · 1 setup item" for a found tool, or why it is
 * empty. Memories are not listed: the agent keeps using its own memory, so
 * the import leaves them where they are. When a tool Bridge imports keeps
 * chats in more than one folder (your own and an account's in Settings),
 * it says how many, so an account's chats are visibly included.
 */
export function sourceSummary(source: DetectedImportSource): string {
  if (!source.installed) return "Not found on this Mac";
  const { chats, projects, workspaces, setup } = source.counts;
  const background = source.counts.background ?? 0;
  const parts: string[] = [];
  if (chats > 0) parts.push(`${chats.toLocaleString()} ${chats === 1 ? "chat" : "chats"}${projects > 0 && workspaces === 0 ? ` in ${projects.toLocaleString()} ${projects === 1 ? "project" : "projects"}` : ""}`);
  if (workspaces > 0) parts.push(`${workspaces.toLocaleString()} ${workspaces === 1 ? "workspace" : "workspaces"} in ${projects.toLocaleString()} ${projects === 1 ? "repo" : "repos"}`);
  if (source.supported && chats > 0 && source.roots.length > 1) parts.push(`from ${source.roots.length.toLocaleString()} folders`);
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
  const failed = job.failed > 0 ? ` ${job.failed.toLocaleString()} could not be imported.` : "";
  const workspaces = job.workspaces ?? 0;
  if (job.imported === 0 && workspaces === 0) {
    const already = job.alreadyInBridge > 0 ? ` ${job.alreadyInBridge.toLocaleString()} already in Bridge.` : "";
    return { title: "Import finished", body: `Nothing new came over from ${toolLabel}.${already}${failed}`, failed: false };
  }
  const items = job.imported > 0 ? `${job.imported.toLocaleString()} ${job.imported === 1 ? "item" : "items"}` : "";
  const adopted = workspaces > 0 ? `${workspaces.toLocaleString()} ${workspaces === 1 ? "workspace" : "workspaces"}` : "";
  const parts = [`${[items, adopted].filter(Boolean).join(" and ")} came over from ${toolLabel}`];
  if (job.alreadyInBridge > 0) parts.push(`${job.alreadyInBridge.toLocaleString()} already in Bridge`);
  return { title: "Import finished", body: `${parts.join(", ")}.${failed}`, failed: false };
}

/**
 * How long a running import has left, the way a file copy says it: from the
 * pace so far. Nothing until a few seconds of pace exist to go on.
 */
export function timeLeft(job: Pick<ImportJob, "processed" | "total" | "startedAt">, nowMs: number): string {
  const elapsed = (nowMs - Date.parse(job.startedAt)) / 1000;
  if (job.total <= 0 || job.processed <= 0 || !(elapsed >= 3)) return "Working out the time left";
  const remaining = Math.max(0, job.total - job.processed);
  if (remaining === 0) return "Finishing up";
  const seconds = (remaining / job.processed) * elapsed;
  if (seconds < 60) return "Less than a minute left";
  const minutes = Math.round(seconds / 60);
  if (minutes < 90) return `About ${minutes} ${minutes === 1 ? "minute" : "minutes"} left`;
  const hours = Math.round(minutes / 60);
  return `About ${hours} ${hours === 1 ? "hour" : "hours"} left`;
}
