import { useCallback, useEffect, useRef, useState } from "react";
import { Download, Mic, RotateCcw, Trash2 } from "lucide-react";
import { bridgeApi } from "../../api";
import type { VoiceLocalSetupState, VoiceLocalStatusResult } from "../../protocol/generated/protocol";
import { GhostButton, SettingsBlockRow, SettingsGroup, SettingsPage, SettingsRow, StatusPill, TextButton, type PillTone } from "./kit";

const ACTIVE = new Set<VoiceLocalSetupState>(["downloadingRuntime", "downloadingModel", "installing"]);

const STATE_LABEL: Record<VoiceLocalSetupState, string> = {
  notInstalled: "Not installed",
  downloadingRuntime: "Downloading engine",
  downloadingModel: "Downloading model",
  installing: "Installing",
  ready: "Ready",
  failed: "Needs attention",
  unsupported: "Unsupported",
};

function tone(state: VoiceLocalSetupState): PillTone {
  if (state === "ready") return "success";
  if (state === "failed") return "destructive";
  if (state === "unsupported") return "warning";
  if (ACTIVE.has(state)) return "info";
  return "neutral";
}

function bytes(value: number): string {
  const mib = value / (1024 * 1024);
  return `${mib >= 100 ? Math.round(mib) : mib.toFixed(1)} MiB`;
}

export function VoiceSettingsPage({ onError, onChanged }: {
  onError: (message: string) => void;
  onChanged?: () => void;
}) {
  const [status, setStatus] = useState<VoiceLocalStatusResult>();
  const [busy, setBusy] = useState(false);
  const previousState = useRef<VoiceLocalSetupState>();
  const changed = useRef(onChanged);
  changed.current = onChanged;

  const refresh = useCallback(async () => {
    try {
      const next = await bridgeApi.voiceLocalStatus();
      setStatus(next);
      if (previousState.current && previousState.current !== next.state && (next.state === "ready" || next.state === "notInstalled")) changed.current?.();
      previousState.current = next.state;
      return next;
    } catch (error) {
      onError(String(error));
      return undefined;
    }
  }, [onError]);

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    if (!status || !ACTIVE.has(status.state)) return;
    const timer = window.setInterval(() => void refresh(), 750);
    return () => window.clearInterval(timer);
  }, [refresh, status]);

  const setup = async () => {
    setBusy(true);
    try { setStatus(await bridgeApi.voiceLocalSetup()); }
    catch (error) { onError(String(error)); }
    finally { setBusy(false); }
  };

  const remove = async () => {
    if (!window.confirm("Remove the local dictation engine and English model from this Mac?")) return;
    setBusy(true);
    try { setStatus(await bridgeApi.voiceLocalRemove()); changed.current?.(); }
    catch (error) { onError(String(error)); }
    finally { setBusy(false); }
  };

  const active = !!status && ACTIVE.has(status.state);
  const progress = status && status.downloadBytes > 0
    ? Math.min(100, Math.round((status.downloadedBytes / status.downloadBytes) * 100))
    : 0;

  return <SettingsPage title="Voice" description="Private, on-device dictation for the composer. Bridge never starts a download until you choose it.">
    <SettingsGroup label="Local dictation">
      <SettingsRow
        label="English speech model"
        description="NVIDIA Nemotron Speech Streaming · English (en-US) · audio stays on this Mac"
        lead={<Mic size={14} strokeWidth={1.7} aria-hidden="true" />}
        control={status ? <StatusPill tone={tone(status.state)}>{STATE_LABEL[status.state]}</StatusPill> : <StatusPill>Checking…</StatusPill>}
      />
      <SettingsRow label="Download" description={status ? `${bytes(status.downloadBytes)} compressed` : "About 460 MiB compressed"} />
      <SettingsRow label="Installed size" description={status ? `About ${bytes(status.installedBytes)}` : "About 662 MiB"} />
      <SettingsRow label="Engine" description={status ? `sherpa-onnx ${status.engineVersion}` : "sherpa-onnx 1.13.8"} mono />
      <SettingsRow label="Model license" description="NVIDIA Open Model License. Review the model terms before use." />
      {status?.reason && <SettingsBlockRow label={status.state === "failed" ? "Setup failed" : "Availability"}>
        <p role={status.state === "failed" ? "alert" : undefined} className={status.state === "failed" ? "text-xs leading-relaxed text-destructive" : "text-xs leading-relaxed text-muted-foreground"}>{status.reason}</p>
      </SettingsBlockRow>}
      {active && <SettingsBlockRow label={STATE_LABEL[status.state]} description={`${bytes(status.downloadedBytes)} of ${bytes(status.downloadBytes)}`}>
        <div className="h-1.5 overflow-hidden rounded-full bg-muted" role="progressbar" aria-label="Local dictation setup" aria-valuenow={progress} aria-valuemin={0} aria-valuemax={100}>
          <div className="h-full rounded-full bg-foreground transition-[width]" style={{ width: `${progress}%` }} />
        </div>
      </SettingsBlockRow>}
      <SettingsBlockRow>
        <div className="flex flex-wrap items-center gap-2">
          {status?.state === "ready"
            ? <TextButton tone="destructive" disabled={busy} onClick={() => void remove()}><Trash2 size={12} strokeWidth={1.7} aria-hidden="true" />Remove local model</TextButton>
            : <GhostButton disabled={busy || active || status?.state === "unsupported"} onClick={() => void setup()}>
                {status?.state === "failed" ? <RotateCcw size={12} strokeWidth={1.7} aria-hidden="true" /> : <Download size={12} strokeWidth={1.7} aria-hidden="true" />}
                {status?.state === "failed" ? "Retry download" : "Download and install"}
              </GhostButton>}
          <span className="text-[11px] text-muted-foreground">No account or API key required.</span>
        </div>
      </SettingsBlockRow>
    </SettingsGroup>
  </SettingsPage>;
}
