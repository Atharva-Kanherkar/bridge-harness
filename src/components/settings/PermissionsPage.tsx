// Permissions: provider convenience, host authorization, and the decision log.
//
// The switch renders from what the host stored, never from what was clicked. It
// is a security control, so showing it on because a request was sent would be a
// lie the moment the request failed.

import type { ReactNode } from "react";
import { Lock as LockSimple } from "lucide-react";
import type { BridgeEvent, PermissionPolicy } from "../../types";
import { SettingsGroup, SettingsPage, SettingsRow, Switch } from "./kit";

/// Host gates that outlive the provider convenience switch.
///
/// Named in the UI, not only in a doc comment, because the issue makes the copy
/// part of the contract: a switch that claims to silence everything and then
/// still prompts has to say up front where and why. Worker write scope is not
/// here: Full access authorizes it like any other approval.
const SURVIVING_GATES = [
  {
    title: "Sending or publishing from a browser",
    copy: "Send, submit, purchase, publish, and credential steps in the browser still ask, every time.",
  },
  {
    title: "Changes to shared agent instructions",
    copy: "Every proposed change to a shared role prompt needs your review of the exact before and after text.",
  },
];

const WORKER_ROLES = ["research", "implementation", "verification", "planning", "documentation"] as const;

export function PermissionsSection({ policy, autoApprovals, busy, saved, onChange, extra }: {
  extra?: ReactNode;
  policy: PermissionPolicy;
  autoApprovals: BridgeEvent[];
  busy: boolean;
  saved?: boolean;
  onChange: (next: PermissionPolicy) => void;
}) {
  const on = policy.autoApproveProviderPermissions === true;
  const proposalRoles = policy.workerPromptProposalRoles ?? [];
  return <SettingsPage title="Permissions" description="How much Bridge asks before an agent acts.">
    <SettingsGroup label="Agent actions">
      <SettingsRow
        label="Let agents act without asking"
        description="Automatically accept supported requests from coding agents and let background tasks write to their assigned files. Agents can run commands and change files without asking. Questions and macOS permissions still need your response."
        saved={saved}
        control={<Switch
          label="Let agents act without asking"
          checked={on}
          disabled={busy}
          onChange={next => onChange({ ...policy, autoApproveProviderPermissions: next })}
        />}
      />
    </SettingsGroup>

    <details className="text-ui"><summary className="cursor-pointer text-muted-foreground">Advanced: suggested instruction changes</summary><div className="mt-3"><SettingsGroup label="Suggested instruction changes" note="Off by default; each change still asks">
      <SettingsRow label="Let background agents suggest improvements to their instructions"
        description="Each agent may suggest changes to its instructions. You review the exact change before it is saved. It applies the next time that role starts." />
      {WORKER_ROLES.map(role => <SettingsRow
        key={role}
        label={<span className="capitalize">{role}</span>}
        control={<Switch
          label={`Allow ${role} prompt proposals`}
          checked={proposalRoles.includes(role)}
          disabled={busy}
          onChange={enabled => onChange({ ...policy, workerPromptProposalRoles: enabled
            ? [...proposalRoles.filter(item => item !== role), role]
            : proposalRoles.filter(item => item !== role) })}
        />}
      />)}
    </SettingsGroup></div></details>

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
    {extra}
  </SettingsPage>;
}
