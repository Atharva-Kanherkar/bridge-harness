// Extra sign-ins for Claude Code and Codex, each picked per chat from the
// model picker. Adapted from T3 Code's "Add account" flow and its provider
// docs (https://github.com/pingdotgg/t3code, MIT; see THIRD_PARTY_NOTICES.md):
// the user signs the second subscription in once from a terminal with the
// provider's own home variable, then names the folder here.

import { useState } from "react";
import {
  ACCOUNT_PROVIDERS, accountIdFromLabel, folderError, loginCommand, suggestedFolder, type ProviderAccount,
} from "../../providerAccounts";
import {
  Field, GhostButton, PrimaryButton, SettingsBlockRow, SettingsGroup, SettingsRow, StatusPill, Switch, TextButton,
} from "./kit";

type AccountHarness = keyof typeof ACCOUNT_PROVIDERS;

export function ProviderAccountsSettings({ harness, label, accounts, disabled, onChange }: {
  harness: AccountHarness;
  label: string;
  accounts: ProviderAccount[];
  disabled?: boolean;
  onChange: (next: ProviderAccount[]) => Promise<void>;
}) {
  const [adding, setAdding] = useState(false);
  const [saving, setSaving] = useState(false);
  const busy = disabled || saving;

  const save = async (next: ProviderAccount[]) => {
    setSaving(true);
    try { await onChange(next); return true; }
    catch { return false; }
    finally { setSaving(false); }
  };

  return <SettingsGroup label="Accounts" note="Use another subscription, chosen per chat">
    <SettingsRow label="Your own login" description={ACCOUNT_PROVIDERS[harness].ownLogin} mono control={<StatusPill>Default</StatusPill>} />
    {accounts.map(account => <SettingsRow
      key={account.id}
      label={account.label}
      description={folderDescription(harness, account)}
      mono
      control={<TextButton tone="destructive" disabled={busy} ariaLabel={`Remove ${account.label}`}
        onClick={() => void save(accounts.filter(item => item.id !== account.id))}>Remove</TextButton>}
    />)}
    {adding
      ? <AddAccountForm harness={harness} label={label} accounts={accounts} disabled={busy}
          onCancel={() => setAdding(false)}
          onAdd={async account => { if (await save([...accounts, account])) setAdding(false); }} />
      : <SettingsRow
          label="Add an account"
          description={`Sign a second ${label} subscription in once, then pick it from the model picker in any chat.`}
          control={<GhostButton disabled={busy} onClick={() => setAdding(true)}>Add account</GhostButton>}
        />}
  </SettingsGroup>;
}

function folderDescription(harness: AccountHarness, account: ProviderAccount): string {
  if (ACCOUNT_PROVIDERS[harness].sharedHome && account.shadowHomePath) {
    return `${account.shadowHomePath} · shares chats with ${account.homePath || "your main login"}`;
  }
  return account.homePath ?? "";
}

function AddAccountForm({ harness, label, accounts, disabled, onAdd, onCancel }: {
  harness: AccountHarness;
  label: string;
  accounts: ProviderAccount[];
  disabled?: boolean;
  onAdd: (account: ProviderAccount) => Promise<void>;
  onCancel: () => void;
}) {
  const [name, setName] = useState("Personal");
  const id = accountIdFromLabel(name, harness, accounts.map(account => account.id));
  const [folderEdited, setFolderEdited] = useState<string | null>(null);
  const folder = folderEdited ?? suggestedFolder(harness, id);
  // Codex can share one home's sessions and settings between logins, so a chat
  // can move to the other account and keep its thread. Claude cannot.
  const [shared, setShared] = useState(true);
  const [copied, setCopied] = useState(false);
  const provider = ACCOUNT_PROVIDERS[harness];

  const nameError = name.trim() ? null : "Give the account a name.";
  const pathError = folderError(folder);
  const account: ProviderAccount = provider.sharedHome && shared
    ? { id, label: name.trim(), shadowHomePath: folder.trim() }
    : { id, label: name.trim(), homePath: folder.trim() };
  const command = loginCommand(harness, account);
  const error = nameError ?? pathError;

  return <SettingsBlockRow label="Add an account" description={`Each account signs in from its own folder. Your own ${label} login is not changed.`}>
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="w-36 shrink-0 text-[12px] text-muted-foreground">Name</span>
        <Field label="Account name" value={name} disabled={disabled} placeholder="e.g. Personal or Work" onChange={setName} />
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <span className="w-36 shrink-0 text-[12px] text-muted-foreground">{provider.variable}</span>
        <Field label="Account folder" value={folder} disabled={disabled} mono placeholder={suggestedFolder(harness, "work")} onChange={value => setFolderEdited(value)} />
      </div>
      {provider.sharedHome && <div className="flex flex-wrap items-center gap-3">
        <span className="min-w-0 flex-1 text-[12px] leading-relaxed text-muted-foreground">
          <span className="block text-[13px] text-foreground">Share chats with your main Codex</span>
          {shared
            ? "Keeps this login separate but shares sessions and settings, so a chat can switch accounts and keep its thread."
            : "A separate Codex home with its own sessions and settings. Switching a chat to it starts a fresh thread."}
        </span>
        <Switch label="Share chats with your main Codex" checked={shared} disabled={disabled} onChange={setShared} />
      </div>}
      <div className="grid gap-1.5">
        <span className="text-[12px] text-muted-foreground">Sign in once in a terminal, choosing the other account in the browser:</span>
        <div className="flex items-start gap-2 rounded-lg border border-border-card bg-popover px-3 py-2">
          <code className="min-w-0 flex-1 break-all font-mono text-[11px] text-foreground select-all" aria-label="Sign-in command">{command}</code>
          <TextButton ariaLabel="Copy sign-in command" onClick={() => {
            void navigator.clipboard?.writeText(command).then(() => setCopied(true), () => setCopied(false));
          }}>{copied ? "Copied" : "Copy"}</TextButton>
        </div>
        {provider.loginNote && <span className="text-[11px] text-muted-foreground">{provider.loginNote}</span>}
      </div>
      {error && <p role="alert" className="text-[12px] text-destructive">{error}</p>}
      <div className="flex items-center justify-end gap-2">
        <TextButton disabled={disabled} onClick={onCancel}>Cancel</TextButton>
        <PrimaryButton disabled={disabled || !!error} onClick={() => void onAdd(account)}>Add account</PrimaryButton>
      </div>
    </div>
  </SettingsBlockRow>;
}
