import { Check, ChevronDown, Shield, ShieldCheck } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuItem, MenuPanel, useMenuPanel } from "@/components/ui/menu-panel";
import type { PermissionPolicy } from "../types";

/**
 * The composer's access control: how agent approvals are handled for every
 * chat, chosen where the work happens rather than announced as a warning in
 * the chrome. Two modes, named for what the user gets:
 *
 * - **Full access** — every provider permission is granted automatically.
 * - **User approval** — Bridge asks before an agent acts.
 *
 * Quiet by design. A mode is a setting, not an alarm, so it wears the same
 * ink as the model control next to it. Renders a disabled placeholder until the
 * policy has loaded so the composer row does not jump.
 */
export type AccessMode = "full" | "ask";

export function accessModeOf(policy: PermissionPolicy | undefined): AccessMode | null {
  if (!policy) return null;
  return policy.autoApproveProviderPermissions ? "full" : "ask";
}

const LABELS: Record<AccessMode, { label: string; detail: string }> = {
  full: { label: "Full access", detail: "Agents act without asking. Approvals are granted for you." },
  ask: { label: "User approval", detail: "Bridge asks you before an agent runs something that needs permission." },
};

export function AccessControl({ policy, disabled, onChange }: {
  policy: PermissionPolicy | undefined;
  disabled?: boolean;
  onChange: (mode: AccessMode) => void;
}) {
  const menu = useMenuPanel<HTMLButtonElement>({ width: 220, height: 88 });
  const mode = accessModeOf(policy);
  const current = mode ? LABELS[mode] : { label: "Access", detail: "" };
  const Icon = mode === "full" ? ShieldCheck : Shield;
  return <>
    <button
      ref={menu.triggerRef}
      type="button"
      disabled={disabled || mode == null}
      onClick={menu.toggle}
      aria-haspopup="menu"
      aria-expanded={menu.open}
      aria-label={`Access: ${current.label}`}
      title={current.detail || undefined}
      className={cn("flex h-8 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground transition-colors enabled:hover:bg-accent disabled:cursor-default disabled:opacity-60")}
    >
      <Icon size={13} aria-hidden="true" className="shrink-0" />
      <span className="whitespace-nowrap">{current.label}</span>
      <ChevronDown size={14} className={cn("shrink-0 transition-transform", menu.open && "rotate-180")} aria-hidden="true" />
    </button>
    <MenuPanel controller={menu} label="Access mode">
      {(["full", "ask"] as AccessMode[]).map(option => <MenuItem
        key={option}
        role="menuitemradio"
        checked={option === mode}
        label={LABELS[option].label}
        leading={option === "full" ? <ShieldCheck size={13} aria-hidden="true" /> : <Shield size={13} aria-hidden="true" />}
        trailing={option === mode ? <Check size={13} aria-hidden="true" /> : undefined}
        onClick={() => { if (option !== mode) onChange(option); menu.close(); }}
      />)}
    </MenuPanel>
  </>;
}
