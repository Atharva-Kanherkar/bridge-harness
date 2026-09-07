import { ShieldOff } from "lucide-react";

/**
 * The standing reminder that approvals are being granted automatically.
 *
 * A mode that silences every prompt must never be invisible, so this lives in
 * the app chrome rather than on the settings page that turned it on — the chrome
 * is the one surface the user is always looking at. It is a button because the
 * way out should be one click from wherever they noticed it, not a hunt through
 * settings.
 *
 * Renders nothing when the policy is off, so callers can mount it
 * unconditionally.
 */
export function BypassBadge({ bypassing, onOpenSettings }: {
  bypassing: boolean;
  onOpenSettings: () => void;
}) {
  if (!bypassing) return null;
  return <button
    type="button"
    onClick={onOpenSettings}
    className="inline-flex h-7 items-center gap-1.5 rounded-full border border-warning/30 bg-warning/10 px-2.5 text-[11px] font-medium text-warning transition-colors hover:bg-warning/20"
    title="Every agent's approvals are being accepted automatically. Click to change."
  >
    <ShieldOff size={11} aria-hidden="true" /> Approvals bypassed
  </button>;
}
