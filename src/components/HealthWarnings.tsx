import { TriangleAlert, X } from "lucide-react";
import { useState } from "react";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import type { HealthWarning } from "../types";

/** Warnings the user has closed, keyed by the backend's stable warning id.
 * Detection keeps firing — an ad-hoc signature is still an ad-hoc signature on
 * the next rebuild — so the dismissal has to outlive the process to be worth
 * anything. */
export const DISMISSED_WARNINGS_KEY = "bridge.health.dismissedWarnings";

export function readDismissedWarnings(): Set<string> {
  if (typeof localStorage === "undefined") return new Set();
  let parsed: unknown;
  try {
    parsed = JSON.parse(localStorage.getItem(DISMISSED_WARNINGS_KEY) ?? "[]");
  } catch {
    return new Set();
  }
  return new Set(Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : []);
}

export function writeDismissedWarnings(ids: Set<string>): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(DISMISSED_WARNINGS_KEY, JSON.stringify([...ids]));
  } catch {
    // A read-only storage should never stop the warning from closing.
  }
}

/** Environment warnings from `health/health` — conditions outside Bridge's
 * process (macOS TCC-protected project folders, ad-hoc code signing) whose
 * shared symptom is file-access prompts that keep coming back. The backend
 * owns detection and the guidance text; this renders nothing when clean or
 * when every warning it reports has already been dismissed. */
export function HealthWarnings({ warnings, className }: { warnings: HealthWarning[]; className?: string }) {
  const [dismissed, setDismissed] = useState(readDismissedWarnings);
  const dismiss = (id: string) => setDismissed(previous => {
    const next = new Set(previous).add(id);
    writeDismissedWarnings(next);
    return next;
  });

  const visible = warnings.filter(warning => !dismissed.has(warning.id));
  if (visible.length === 0) return null;
  return <>{visible.map(warning => <Alert key={warning.id} variant="warning" className={className}>
    <TriangleAlert size={16} aria-hidden="true" />
    <AlertTitle>{warning.title}</AlertTitle>
    <AlertDescription>
      <p>{warning.detail}</p>
      {warning.paths.length > 0 && <ul className="mt-1.5 space-y-0.5">
        {warning.paths.map(path => <li key={path} className="truncate font-mono text-[11px]" title={path}>{path}</li>)}
      </ul>}
    </AlertDescription>
    <AlertAction>
      <Button type="button" size="icon-sm" variant="ghost" aria-label="Dismiss warning" onClick={() => dismiss(warning.id)}><X size={14} aria-hidden="true" /></Button>
    </AlertAction>
  </Alert>)}</>;
}
