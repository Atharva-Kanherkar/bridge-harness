import { useEffect, useState } from "react";
import { KeyRound } from "lucide-react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import type { ApiKeysSnapshot, ChatApiKey, SavedApiKey } from "../../types";
import { DEFAULT_KEY_NAMES, DEFAULT_KEY_SITES, guessKeyKind, isValidKeyName, keyKindLabel, parseSites, toKeyName } from "../../apiKeys";
import { Field, PrimaryButton, SettingsBlockRow, SettingsGroup, SettingsPage, SettingsRow, StatusPill, TextButton, useSavedFlash } from "./kit";

// API keys: the keys an agent can use without ever reading them. The agent
// holds a placeholder; Bridge puts the real key in the request header, only on
// the sites listed here. Every action persists on the spot, like Browser
// copies. No value ever comes back from the native side, so nothing here can
// show one.

const sitesLabel = (hosts: string[]) => hosts.length ? hosts.join(", ") : "No sites yet, so agents cannot use it";

export function KeysPage({ onError }: { onError: (message: string) => void }) {
  const [snapshot, setSnapshot] = useState<ApiKeysSnapshot>();
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<string>();
  const [repasting, setRepasting] = useState<string>();
  const [promoting, setPromoting] = useState<string>();
  const [isFlashed, flash] = useSavedFlash();

  useEffect(() => {
    let active = true;
    bridgeApi.listApiKeys()
      .then(next => { if (active) setSnapshot(next); })
      .catch(error => onError(error instanceof Error ? error.message : String(error)));
    return () => { active = false; };
  }, [onError]);

  const run = async (work: () => Promise<ApiKeysSnapshot>, key?: string): Promise<boolean> => {
    setBusy(true);
    try {
      setSnapshot(await work());
      if (key) flash(key);
      return true;
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      setBusy(false);
    }
  };

  if (!snapshot) {
    return <SettingsPage title="API keys">
      <SkeletonRows label="Loading API keys" rows={3} lead="icon" trailing />
    </SettingsPage>;
  }

  return <SettingsPage
    title="API keys"
    description="Agents can call APIs with these keys without ever seeing them. Each key is sent only to the sites you allow."
  >
    {!snapshot.proxyRunning && <SettingsGroup>
      <SettingsRow label="Agents cannot use keys right now" description="Bridge could not open the local port agents call through. Restart Bridge to try again." control={<StatusPill tone="warning">Off</StatusPill>} />
    </SettingsGroup>}

    <SettingsGroup label="Saved keys" note={snapshot.canSave ? "Kept in your macOS Keychain" : "Saving needs the macOS Keychain"}>
      {snapshot.saved.length === 0 && <SettingsRow label="No saved keys yet" description="Add one below, or save a key you pasted into a chat." />}
      {snapshot.saved.map(key => editing === key.name
        ? <SitesEditor
            key={key.name}
            name={key.name}
            initial={key.hosts}
            busy={busy}
            onCancel={() => setEditing(undefined)}
            onSave={async hosts => { if (await run(() => bridgeApi.saveApiKey({ name: key.name, hosts }), key.name)) setEditing(undefined); }}
          />
        : repasting === key.name
        ? <RepasteForm
            key={key.name}
            name={key.name}
            busy={busy}
            onCancel={() => setRepasting(undefined)}
            onSave={async value => { if (await run(() => bridgeApi.saveApiKey({ name: key.name, value, hosts: key.hosts }), key.name)) setRepasting(undefined); }}
          />
        : <SavedKeyRow
            key={key.name}
            apiKey={key}
            busy={busy}
            saved={isFlashed(key.name)}
            onEdit={() => setEditing(key.name)}
            onRepaste={() => setRepasting(key.name)}
            onForget={() => void run(() => bridgeApi.forgetApiKey(key.name))}
          />)}
    </SettingsGroup>

    {snapshot.canSave && <SettingsGroup label="Add a key">
      <AddKeyForm busy={busy} taken={snapshot.saved.map(key => key.name)} onSave={(name, value, hosts) => run(() => bridgeApi.saveApiKey({ name, value, hosts }), name)} />
    </SettingsGroup>}

    <SettingsGroup label="Pasted in chats" note="Forgotten when Bridge quits">
      {snapshot.chat.length === 0 && <SettingsRow label="Nothing pasted" description="When you paste a key into a chat, Bridge hides it from the agent and lists it here." />}
      {snapshot.chat.map(key => promoting === key.reference
        ? <PromoteForm
            key={key.reference}
            chatKey={key}
            busy={busy}
            onCancel={() => setPromoting(undefined)}
            onSave={async (name, hosts) => { if (await run(() => bridgeApi.saveApiKey({ name, hosts, fromReference: key.reference }), name)) setPromoting(undefined); }}
          />
        : <ChatKeyRow
            key={key.reference}
            chatKey={key}
            busy={busy}
            canSave={snapshot.canSave}
            onSave={() => setPromoting(key.reference)}
            onForget={() => void run(() => bridgeApi.forgetApiKey(key.reference))}
          />)}
    </SettingsGroup>

    <SettingsGroup label="How agents use a key">
      <SettingsRow
        label="The agent only sees a placeholder"
        description="It writes [secret:OPENAI_API_KEY] where the key belongs. When it calls an allowed site through Bridge, Bridge puts the real key in the request header and removes it from the reply. Any other site is refused."
      />
    </SettingsGroup>
  </SettingsPage>;
}

function SavedKeyRow({ apiKey, busy, saved, onEdit, onRepaste, onForget }: { apiKey: SavedApiKey; busy: boolean; saved: boolean; onEdit: () => void; onRepaste: () => void; onForget: () => void }) {
  return <SettingsRow
    lead={<KeyRound size={14} aria-hidden="true" />}
    label={<span className="font-mono text-[12px]">{apiKey.name}</span>}
    description={apiKey.needsValue
      ? "This build of Bridge cannot read the saved value, so agents cannot use it. Paste the key again."
      : `${keyKindLabel(apiKey.kind)} · ${sitesLabel(apiKey.hosts)}`}
    saved={saved}
    control={<>
      {apiKey.needsValue
        ? <TextButton onClick={onRepaste} disabled={busy} ariaLabel={`Paste ${apiKey.name} again`}>Paste again</TextButton>
        : <TextButton onClick={onEdit} disabled={busy} ariaLabel={`Edit sites for ${apiKey.name}`}>Edit sites</TextButton>}
      <TextButton tone="destructive" onClick={onForget} disabled={busy} ariaLabel={`Forget ${apiKey.name}`}>Forget</TextButton>
    </>}
  />;
}

function ChatKeyRow({ chatKey, busy, canSave, onSave, onForget }: { chatKey: ChatApiKey; busy: boolean; canSave: boolean; onSave: () => void; onForget: () => void }) {
  const label = keyKindLabel(chatKey.kind);
  return <SettingsRow
    lead={<KeyRound size={14} aria-hidden="true" />}
    label={<>{label}{chatKey.sessionLabel && <span className="text-muted-foreground"> in {chatKey.sessionLabel}</span>}</>}
    description={chatKey.savedAs ? `Saved as ${chatKey.savedAs}` : sitesLabel(chatKey.hosts)}
    control={<>
      {chatKey.savedAs
        ? <StatusPill tone="success">Saved</StatusPill>
        : canSave && <TextButton onClick={onSave} disabled={busy} ariaLabel={`Save this ${label} for every chat`}>Save for every chat</TextButton>}
      <TextButton tone="destructive" onClick={onForget} disabled={busy} ariaLabel={`Forget this ${label}`}>Forget</TextButton>
    </>}
  />;
}

function SitesEditor({ name, initial, busy, onCancel, onSave }: { name: string; initial: string[]; busy: boolean; onCancel: () => void; onSave: (hosts: string[]) => void }) {
  const [sites, setSites] = useState(initial.join(", "));
  return <SettingsBlockRow label={<span className="font-mono text-[12px]">{name}</span>} description="Sites this key may be sent to. Separate them with commas; *.example.com covers its subdomains.">
    <div className="flex flex-wrap items-center gap-2">
      <Field label={`Sites for ${name}`} value={sites} onChange={setSites} width="w-80" mono placeholder="api.example.com" />
      <PrimaryButton disabled={busy} onClick={() => onSave(parseSites(sites))}>Save</PrimaryButton>
      <TextButton onClick={onCancel} disabled={busy}>Cancel</TextButton>
    </div>
  </SettingsBlockRow>;
}

function RepasteForm({ name, busy, onCancel, onSave }: { name: string; busy: boolean; onCancel: () => void; onSave: (value: string) => void }) {
  const [value, setValue] = useState("");
  return <SettingsBlockRow label={<span className="font-mono text-[12px]">{name}</span>} description="Its name and sites stay as they are.">
    <div className="flex flex-wrap items-center gap-2">
      <Field label={`New value for ${name}`} type="password" value={value} onChange={setValue} width="w-80" placeholder="Paste the key" />
      <PrimaryButton disabled={busy || value.trim() === ""} onClick={() => onSave(value)}>Save</PrimaryButton>
      <TextButton onClick={onCancel} disabled={busy}>Cancel</TextButton>
    </div>
  </SettingsBlockRow>;
}

function PromoteForm({ chatKey, busy, onCancel, onSave }: { chatKey: ChatApiKey; busy: boolean; onCancel: () => void; onSave: (name: string, hosts: string[]) => void }) {
  const [name, setName] = useState(DEFAULT_KEY_NAMES[chatKey.kind] ?? "API_KEY");
  const [sites, setSites] = useState(chatKey.hosts.join(", "));
  const valid = isValidKeyName(name);
  return <SettingsBlockRow label={`Save this ${keyKindLabel(chatKey.kind)} for every chat`} description="Agents in any chat will write [secret:NAME] to use it.">
    <div className="flex flex-wrap items-center gap-2">
      <Field label="Name to save it as" value={name} onChange={next => setName(toKeyName(next))} width="w-48" mono />
      <Field label="Sites to allow" value={sites} onChange={setSites} width="w-64" mono placeholder="api.example.com" />
      <PrimaryButton disabled={busy || !valid} onClick={() => onSave(name, parseSites(sites))}>Save</PrimaryButton>
      <TextButton onClick={onCancel} disabled={busy}>Cancel</TextButton>
    </div>
  </SettingsBlockRow>;
}

function AddKeyForm({ busy, taken, onSave }: { busy: boolean; taken: string[]; onSave: (name: string, value: string, hosts: string[]) => Promise<boolean> }) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [sites, setSites] = useState("");
  const [touched, setTouched] = useState({ name: false, sites: false });
  const valid = isValidKeyName(name) && value.trim() !== "" && parseSites(sites).length > 0;
  const replacing = taken.includes(name);

  // A recognizable key fills in its usual name and site, until the person
  // types their own.
  const changeValue = (next: string) => {
    setValue(next);
    const kind = guessKeyKind(next);
    if (!touched.name) setName(DEFAULT_KEY_NAMES[kind] ?? "");
    if (!touched.sites) setSites((DEFAULT_KEY_SITES[kind] ?? []).join(", "));
  };
  const save = async () => {
    if (await onSave(name, value, parseSites(sites))) {
      setName(""); setValue(""); setSites(""); setTouched({ name: false, sites: false });
    }
  };

  return <SettingsBlockRow description="The value goes straight to the Keychain. Bridge never shows it again, and no agent ever sees it.">
    <div className="grid gap-2 @md/settings:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <Field label="Key value" type="password" value={value} onChange={changeValue} width="w-full" placeholder="Paste the key" />
      <Field label="Key name" value={name} onChange={next => { setTouched(current => ({ ...current, name: true })); setName(toKeyName(next)); }} width="w-full" mono placeholder="OPENAI_API_KEY" />
      <Field label="Sites" value={sites} onChange={next => { setTouched(current => ({ ...current, sites: true })); setSites(next); }} width="w-full" mono placeholder="api.example.com" />
      <div className="flex items-center gap-2">
        <PrimaryButton disabled={busy || !valid} onClick={() => void save()}>{replacing ? "Replace key" : "Save key"}</PrimaryButton>
        {replacing && <span className="text-[12px] text-muted-foreground">Replaces the saved {name}</span>}
      </div>
    </div>
  </SettingsBlockRow>;
}
