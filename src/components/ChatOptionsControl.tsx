import { useState } from "react";
import { Settings2 } from "lucide-react";
import { Select } from "./settings/kit";
import { bridgeApi } from "../api";
import { defaultsFromSettings, settingSourceLabel } from "../chatDefaults";
import type { ActiveTurnInput, ResolvedChatSettings } from "../protocol/generated/protocol";

export function ChatOptionsControl({ settings, disabled, onInputChange, onContinue, onOpenDefaults }: {
  settings?: ResolvedChatSettings; disabled?: boolean; onInputChange: (value: ActiveTurnInput) => void; onContinue?: () => void; onOpenDefaults: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string>();
  const save = async (projectId: string | null) => {
    if (!settings) return;
    setBusy(true); setNotice(undefined);
    try {
      const view = await bridgeApi.getChatDefaults(projectId);
      await bridgeApi.saveChatDefaults({ projectId, expectedRevision: (projectId ? view.project : view.application)?.revision ?? 0, defaults: defaultsFromSettings(settings) });
      setNotice(projectId ? "Saved for future chats in this project" : "Saved for future chats");
    } catch (error) { setNotice(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  return <details className="relative shrink-0">
    <summary aria-label="Chat settings" className="flex h-8 cursor-pointer list-none items-center gap-1 rounded-md px-2 text-xs text-muted-foreground hover:bg-accent"><Settings2 size={14} aria-hidden="true" /><span className="sr-only">Chat settings</span></summary>
    <div className="u-glass-popover absolute bottom-full right-0 z-30 mb-2 flex w-72 flex-col gap-2 rounded-xl border border-border p-3 text-ui">
      <div className="flex items-center justify-between gap-2"><span>While running</span>
        <Select label="Chat input while running" disabled={disabled || busy || !settings} value={settings?.activeTurnInput ?? "steer"} options={[{value:"steer",label:"Steer"},{value:"queue",label:"Queue"}]} width="w-24" onChange={value => onInputChange(value as ActiveTurnInput)} />
      </div>
      {settings && <p className="text-xs text-muted-foreground">Model: {settingSourceLabel(settings.sources.model)} · Access: {settingSourceLabel(settings.sources.access)}</p>}
      {onContinue && <button type="button" onClick={onContinue} className="rounded-lg px-2 py-1.5 text-left hover:bg-accent">Change agent or mode in a continuation…</button>}
      {settings?.projectId && <button type="button" disabled={disabled || busy} onClick={() => void save(settings.projectId!)} className="rounded-lg px-2 py-1.5 text-left hover:bg-accent disabled:opacity-45">Save as this project's defaults</button>}
      <button type="button" disabled={disabled || busy || !settings} onClick={() => void save(null)} className="rounded-lg px-2 py-1.5 text-left hover:bg-accent disabled:opacity-45">Save as application defaults</button>
      <button type="button" onClick={onOpenDefaults} className="rounded-lg px-2 py-1.5 text-left hover:bg-accent">Edit new chat defaults…</button>
      {notice && <p role="status" className="text-xs text-muted-foreground">{notice}</p>}
    </div>
  </details>;
}
