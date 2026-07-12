import type { QueuedWorkerRequest, RestorationMode, SessionForestSnapshot, WorkerLease } from "./types";

export function restorationPresentation(mode: RestorationMode) {
  if (mode === "hot") return { label: "HOT", detail: "process alive" };
  if (mode === "native") return { label: "NATIVE RESUME", detail: "provider resumed" };
  if (mode === "checkpoint_restored") return { label: "CHECKPOINT RESTORED", detail: "checkpoint replayed" };
  return { label: "FRESH", detail: "new context" };
}

export function turnBudget(snapshot?: SessionForestSnapshot, preferredTurnId?: string | null) {
  const turnId = preferredTurnId ?? [...(snapshot?.usage ?? [])].reverse().find(row => row.turnId)?.turnId ?? null;
  const rows = (snapshot?.usage ?? []).filter(row => row.turnId === turnId);
  return {
    turnId,
    units: rows.reduce((sum, row) => sum + row.capabilityUnits, 0),
    workers: rows.filter(row => row.source.startsWith("policy.spawn.")).length,
    strongWorkers: rows.filter(row => row.source === "policy.spawn.strong").length,
  };
}

export function queueExplanation(item: QueuedWorkerRequest, leases: WorkerLease[]): string {
  const explicit = typeof item.request.reason === "string" ? item.request.reason.replaceAll("_", " ") : "";
  const owned = Array.isArray(item.request.ownedPaths) ? item.request.ownedPaths.filter((value): value is string => typeof value === "string") : [];
  const conflict = leases.find(lease => lease.leaseStatus === "active" && lease.writeMode !== "readOnly" && lease.ownedPaths.some(path => owned.some(candidate => pathsOverlap(path, candidate))));
  if (conflict) return `Waiting: ${conflict.role} owns ${conflict.ownedPaths.join(", ")}.`;
  return explicit ? `Waiting: ${explicit}.` : "Waiting for policy capacity or an active writer lease.";
}

function pathsOverlap(left: string, right: string): boolean {
  const clean = (value: string) => value.replace(/\*\*?$/g, "").replace(/\/$/, "");
  const a = clean(left); const b = clean(right);
  return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}
