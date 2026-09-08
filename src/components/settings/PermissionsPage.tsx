// Permissions: the bypass switch, the two gates that outlive it, and the log.
//
// The switch renders from what the host stored, never from what was clicked. It
// is a security control, so showing it on because a request was sent would be a
// lie the moment the request failed.

import { Lock as LockSimple } from "lucide-react";
import type { BridgeEvent, PermissionPolicy } from "../../types";
import { SettingsGroup, SettingsPage, SettingsRow, Switch } from "./kit";

/// The two gates that outlive the bypass switch.
///
/// Named in the UI, not only in a doc comment, because the issue makes the copy
/// part of the contract: a switch that claims to silence everything and then
/// still prompts has to say up front where and why. Both are authorization
/// rather than convenience: a worker writing outside its lease, and an outward
/// effect like sending or purchasing.
const SURVIVING_GATES = [
  {
    title: "Worker write scope",
    copy: "A worker still needs your authorization for the paths it may write. Bypass covers convenience, not authorization.",
  },
  {
    title: "Browser outward effects",
    copy: "Send, submit, purchase, publish, and credential steps in the browser still ask, every time.",
  },
];

export function PermissionsSection({ policy, autoApprovals, busy, saved, onChange }: {
  policy: PermissionPolicy;
  autoApprovals: BridgeEvent[];
  busy: boolean;
  saved?: boolean;
  onChange: (next: PermissionPolicy) => void;
}) {
  const on = policy.autoApproveProviderPermissions === true;
  return <SettingsPage title="Permissions" description="How much Bridge asks before an agent acts.">
    <SettingsGroup label="Provider prompts">
      <SettingsRow
        label="Auto-approve provider permissions"
        description="Permission requests from Claude, Codex, OpenCode, and Cursor are accepted automatically when the provider offers an allow option. Questions and macOS prompts still wait for you."
        saved={saved}
        control={<Switch
          label="Auto-approve provider permissions"
          checked={on}
          disabled={busy}
          onChange={next => onChange({ ...policy, autoApproveProviderPermissions: next })}
        />}
      />
    </SettingsGroup>

    <SettingsGroup label="Always asks" note="These keep asking either way">
      {SURVIVING_GATES.map(gate => <SettingsRow
        key={gate.title}
        lead={<LockSimple size={12} strokeWidth={1.7} aria-hidden="true" />}
        label={gate.title}
        description={gate.copy}
      />)}
    </SettingsGroup>

    <SettingsGroup label="Recent auto-approvals" note="Every automatic decision is recorded">
      {autoApprovals.length === 0
        ? <SettingsRow label={<span className="font-mono text-[11px] text-muted-foreground">Nothing has been auto-approved yet.</span>} />
        : autoApprovals.map(event => <SettingsRow
            key={event.id}
            label={<span className="font-mono text-[11px] text-foreground/85">{event.body}</span>}
            control={<time className="shrink-0 font-mono text-[11px] text-muted-foreground">
              {new Date(event.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
            </time>}
          />)}
    </SettingsGroup>
  </SettingsPage>;
}
