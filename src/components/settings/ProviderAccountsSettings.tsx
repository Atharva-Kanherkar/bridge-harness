// Extra sign-ins for a coding agent, each picked per chat from the model
// picker. Adapted from T3 Code's "Add account" flow and its provider docs
// (https://github.com/pingdotgg/t3code, MIT; see THIRD_PARTY_NOTICES.md). How
// an account signs in comes from the agent's row in `ACCOUNT_PROVIDERS`: its
// own CLI against the account's folder, a key handed to a private agent
// server for that folder, or an API key Bridge keeps in the macOS Keychain.

import { useState } from "react";
import { bridgeApi } from "../../api";
import {
  ACCOUNT_PROVIDERS, accountIdFromLabel, folderError, loginCommand, suggestedFolder,
  type AccountHarness, type ProviderAccount,
} from "../../providerAccounts";
import {
  Field, GhostButton, PrimaryButton, SettingsBlockRow, SettingsGroup, SettingsRow, StatusPill, Switch, TextButton,
} from "./kit";

/** Hand an account's key to wherever its agent keeps it. */
async function saveAccountKey(harness: AccountHarness, account: string, key: string): Promise<void> {
  const provider = ACCOUNT_PROVIDERS[harness];
  if (provider.signIn === "apiKey") {
    await bridgeApi.setAccountApiKey(harness, account, key);
  } else if (provider.key && "opencodeProvider" in provider.key) {
    await bridgeApi.setOpenCodeProviderApiKey(provider.key.opencodeProvider, key, undefined, account);
  }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function ProviderAccountsSettings({ harness, label, accounts, disabled, onChange }: {
  harness: AccountHarness;
  label: string;
  accounts: ProviderAccount[];
  disabled?: boolean;
  onChange: (next: ProviderAccount[]) => Promise<void>;
}) {
  const provider = ACCOUNT_PROVIDERS[harness];
  const [adding, setAdding] = useState(false);
  const [saving, setSaving] = useState(false);
  const [keyFor, setKeyFor] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedKey, setSavedKey] = useState<string | null>(null);
  const busy = disabled || saving;

  const save = async (next: ProviderAccount[]) => {
    setSaving(true);
    setError(null);
    try { await onChange(next); return true; }
    catch { return false; }
    finally { setSaving(false); }
  };

  const storeKey = async (account: ProviderAccount, key: string) => {
    setSaving(true);
    setError(null);
    try {
      await saveAccountKey(harness, account.id, key);
      setSavedKey(account.id);
      return true;
    } catch (caught) {
      setError(`${account.label}: ${message(caught)}`);
      return false;
    } finally {
      setSaving(false);
    }
  };

  return <SettingsGroup label="Accounts" note="Use another subscription, chosen per chat">
    <SettingsRow label="Your own login" description={provider.ownLogin} mono control={<StatusPill>Default</StatusPill>} />
    {accounts.map(account => keyFor === account.id && provider.key
      ? <KeyEntry key={account.id} label={`${account.label}: ${provider.key.label}`} help={provider.key.help} disabled={busy}
          onCancel={() => setKeyFor(null)}
          onSave={async key => { if (await storeKey(account, key)) setKeyFor(null); }} />
      : <SettingsRow
          key={account.id}
          label={account.label}
          description={accountDescription(harness, account)}
          mono
          saved={savedKey === account.id}
          control={<span className="flex items-center gap-1">
            {provider.key && <TextButton disabled={busy} ariaLabel={`Set ${provider.key.label} for ${account.label}`}
              onClick={() => { setSavedKey(null); setKeyFor(account.id); }}>Set key</TextButton>}
            <TextButton tone="destructive" disabled={busy} ariaLabel={`Remove ${account.label}`}
              onClick={() => void save(accounts.filter(item => item.id !== account.id))}>Remove</TextButton>
          </span>}
        />)}
    {error && <SettingsBlockRow><p role="alert" className="text-[12px] text-destructive">{error}</p></SettingsBlockRow>}
    {adding
      ? <AddAccountForm harness={harness} label={label} accounts={accounts} disabled={busy}
          onCancel={() => setAdding(false)}
          onAdd={async (account, key) => {
            if (!(await save([...accounts, account]))) return;
            setAdding(false);
            if (key) await storeKey(account, key);
          }} />
      : <SettingsRow
          label="Add an account"
          description={`Sign a second ${label} subscription in once, then pick it from the model picker in any chat.`}
          control={<GhostButton disabled={busy} onClick={() => setAdding(true)}>Add account</GhostButton>}
        />}
  </SettingsGroup>;
}

function accountDescription(harness: AccountHarness, account: ProviderAccount): string {
  const provider = ACCOUNT_PROVIDERS[harness];
  if (provider.signIn === "apiKey") return "Signs in with its own API key";
  if (provider.sharedHome && account.shadowHomePath) {
    return `${account.shadowHomePath} · shares chats with ${account.homePath || "your main login"}`;
  }
  return account.homePath ?? "";
}

function KeyEntry({ label, help, disabled, onSave, onCancel }: {
  label: string;
  help: string;
  disabled?: boolean;
  onSave: (key: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [key, setKey] = useState("");
  return <SettingsBlockRow label={label} description={help}>
    <div className="flex flex-wrap items-center gap-2">
      <Field label={label} type="password" value={key} disabled={disabled} mono placeholder="Paste the key" width="w-72" onChange={setKey} />
      <TextButton disabled={disabled} onClick={onCancel}>Cancel</TextButton>
      <PrimaryButton disabled={disabled || !key.trim()} onClick={() => void onSave(key.trim())}>Save key</PrimaryButton>
    </div>
  </SettingsBlockRow>;
}

function AddAccountForm({ harness, label, accounts, disabled, onAdd, onCancel }: {
  harness: AccountHarness;
  label: string;
  accounts: ProviderAccount[];
  disabled?: boolean;
  onAdd: (account: ProviderAccount, key: string) => Promise<void>;
  onCancel: () => void;
}) {
  const provider = ACCOUNT_PROVIDERS[harness];
  const usesFolder = provider.signIn !== "apiKey";
  const [name, setName] = useState("Personal");
  const id = accountIdFromLabel(name, harness, accounts.map(account => account.id));
  const [folderEdited, setFolderEdited] = useState<string | null>(null);
  const folder = folderEdited ?? suggestedFolder(harness, id);
  // Codex can share one home's sessions and settings between logins, so a chat
  // can move to the other account and keep its thread. Others cannot.
  const [shared, setShared] = useState(true);
  const [key, setKey] = useState("");
  // A required key starts empty; say so only once the field has been used,
  // while Add stays disabled either way.
  const [keyTouched, setKeyTouched] = useState(false);
  const [copied, setCopied] = useState(false);

  const account: ProviderAccount = !usesFolder
    ? { id, label: name.trim() }
    : provider.sharedHome && shared
      ? { id, label: name.trim(), shadowHomePath: folder.trim() }
      : { id, label: name.trim(), homePath: folder.trim() };
  const command = loginCommand(harness, account);
  const missingKey = !!provider.key?.required && !key.trim();
  const error = (name.trim() ? null : "Give the account a name.")
    ?? (usesFolder ? folderError(folder) : null)
    ?? (missingKey && keyTouched ? `Paste the ${provider.key?.label}.` : null);
  const description = usesFolder
    ? `Each account signs in from its own folder. Your own ${label} login is not changed.`
    : `Each account signs in with its own API key. Your own ${label} login is not changed.`;

  return <SettingsBlockRow label="Add an account" description={description}>
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="w-36 shrink-0 text-[12px] text-muted-foreground">Name</span>
        <Field label="Account name" value={name} disabled={disabled} placeholder="e.g. Personal or Work" onChange={setName} />
      </div>
      {usesFolder && <div className="flex flex-wrap items-center gap-3">
        <span className="w-36 shrink-0 text-[12px] text-muted-foreground">{provider.variable}</span>
        <Field label="Account folder" value={folder} disabled={disabled} mono placeholder={suggestedFolder(harness, "work")} onChange={value => setFolderEdited(value)} />
      </div>}
      {provider.sharedHome && <div className="flex flex-wrap items-center gap-3">
        <span className="min-w-0 flex-1 text-[12px] leading-relaxed text-muted-foreground">
          <span className="block text-[13px] text-foreground">Share chats with your main {label}</span>
          {shared
            ? "Keeps this login separate but shares sessions and settings, so a chat can switch accounts and keep its thread."
            : `A separate ${label} home with its own sessions and settings. Switching a chat to it starts a fresh thread.`}
        </span>
        <Switch label={`Share chats with your main ${label}`} checked={shared} disabled={disabled} onChange={setShared} />
      </div>}
      {provider.key && <div className="grid gap-1.5">
        <div className="flex flex-wrap items-center gap-3">
          <span className="w-36 shrink-0 text-[12px] text-muted-foreground">{provider.key.required ? "API key" : "API key (optional)"}</span>
          <Field label={provider.key.label} type="password" value={key} disabled={disabled} mono placeholder="Paste the key" width="w-72" onChange={value => { setKey(value); setKeyTouched(true); }} />
        </div>
        <span className="text-[11px] text-muted-foreground">{provider.key.help}</span>
      </div>}
      {command && <div className="grid gap-1.5">
        <span className="text-[12px] text-muted-foreground">Sign in once in a terminal, choosing the other account in the browser:</span>
        <div className="flex items-start gap-2 rounded-lg border border-border-card bg-popover px-3 py-2">
          <code className="min-w-0 flex-1 break-all font-mono text-[11px] text-foreground select-all" aria-label="Sign-in command">{command}</code>
          <TextButton ariaLabel="Copy sign-in command" onClick={() => {
            void navigator.clipboard?.writeText(command).then(() => setCopied(true), () => setCopied(false));
          }}>{copied ? "Copied" : "Copy"}</TextButton>
        </div>
      </div>}
      {provider.loginNote && <span className="text-[11px] text-muted-foreground">{provider.loginNote}</span>}
      {error && <p role="alert" className="text-[12px] text-destructive">{error}</p>}
      <div className="flex items-center justify-end gap-2">
        <TextButton disabled={disabled} onClick={onCancel}>Cancel</TextButton>
        <PrimaryButton disabled={disabled || !!error || missingKey} onClick={() => void onAdd(account, key.trim())}>Add account</PrimaryButton>
      </div>
    </div>
  </SettingsBlockRow>;
}
