import { TriangleAlert } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import type { HealthWarning } from "../types";

/** Environment warnings from `health/health` — conditions outside Bridge's
 * process (macOS TCC-protected project folders, ad-hoc code signing) whose
 * shared symptom is file-access prompts that keep coming back. The backend
 * owns detection and the guidance text; this renders nothing when clean. */
export function HealthWarnings({ warnings, className }: { warnings: HealthWarning[]; className?: string }) {
  if (warnings.length === 0) return null;
  return <>{warnings.map(warning => <Alert key={warning.id} variant="warning" className={className}>
    <TriangleAlert size={16} aria-hidden="true" />
    <AlertTitle>{warning.title}</AlertTitle>
    <AlertDescription>
      <p>{warning.detail}</p>
      {warning.paths.length > 0 && <ul className="mt-1.5 space-y-0.5">
        {warning.paths.map(path => <li key={path} className="truncate font-mono text-[11px]" title={path}>{path}</li>)}
      </ul>}
    </AlertDescription>
  </Alert>)}</>;
}
