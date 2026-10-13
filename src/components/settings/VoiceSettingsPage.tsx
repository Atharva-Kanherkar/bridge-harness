import { useCallback, useEffect, useRef, useState } from "react";
import { Download, Mic, RotateCcw, Trash2 } from "lucide-react";
import { bridgeApi } from "../../api";
import type { VoiceLocalSetupState, VoiceLocalStatusResult } from "../../protocol/generated/protocol";
import { VoiceMicrophoneTest } from "./VoiceMicrophoneTest";
import { GhostButton, SettingsBlockRow, SettingsGroup, SettingsPage, SettingsRow, StatusPill, TextButton, type PillTone } from "./kit";

const ACTIVE = new Set<VoiceLocalSetupState>(["downloadingRuntime", "downloadingModel", "installing"]);
const STATE_LABEL: Record<VoiceLocalSetupState, string> = {
  notInstalled: "Not installed", downloadingRuntime: "Downloading engine", downloadingModel: "Downloading model",
  installing: "Installing", ready: "Ready", failed: "Needs attention", unsupported: "Unsupported",
};
function tone(state: VoiceLocalSetupState): PillTone {
  if (state === "ready") return "success";
  if (state === "failed") return "destructive";
  if (state === "unsupported") return "warning";
  return ACTIVE.has(state) ? "info" : "neutral";
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
  const [loadError, setLoadError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [details, setDetails] = useState(false);
  const previousState = useRef<VoiceLocalSetupState>();
  const latest = useRef({ onChanged, onError });
  latest.current = { onChanged, onError };
  const generation = useRef(0);
  const mounted = useRef(false);
  const mutation = useRef(false);

  const acceptStatus = useCallback((next: VoiceLocalStatusResult) => {
    setStatus(next);
    setLoadError(undefined);
    if (previousState.current !== next.state && (next.state === "ready" || (previousState.current && next.state === "notInstalled"))) latest.current.onChanged?.();
    previousState.current = next.state;
  }, []);

  const refresh = useCallback(async () => {
    if (mutation.current) return;
    const request = ++generation.current;
    try {
      const next = await bridgeApi.voiceLocalStatus();
      if (mounted.current && request === generation.current) acceptStatus(next);
    } catch {
      if (mounted.current && request === generation.current) setLoadError("Could not check local dictation. Try again.");
    }
  }, [acceptStatus]);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    return () => { mounted.current = false; generation.current += 1; };
  }, [refresh]);
  // Schedule the next probe only after the previous one settled. Re-rendering
  // progress cannot reset the timer or create overlapping status requests.
  const active = !!status && ACTIVE.has(status.state);
  useEffect(() => {
    if (!active || busy) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await refresh();
      if (!disposed) timer = setTimeout(() => void poll(), 750);
    };
    timer = setTimeout(() => void poll(), 750);
    return () => { disposed = true; clearTimeout(timer); };
  }, [active, busy, refresh]);

  const mutate = async (operation: "setup" | "remove") => {
    if (mutation.current || testing) return;
    if (operation === "remove" && !window.confirm("Remove the local dictation engine and English model from this Mac?")) return;
    mutation.current = true;
    const request = ++generation.current;
    setBusy(true);
    try {
      const next = await (operation === "setup" ? bridgeApi.voiceLocalSetup() : bridgeApi.voiceLocalRemove());
      if (mounted.current && request === generation.current) acceptStatus(next);
    } catch (error) {
      if (mounted.current) latest.current.onError(String(error));
    } finally {
      mutation.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const progress = status && status.downloadBytes > 0
    ? Math.min(100, Math.round((status.downloadedBytes / status.downloadBytes) * 100)) : 0;

  return <SettingsPage title="Voice" description="Dictate, review, then send. Private English speech recognition on your Mac.">
    <SettingsGroup label="Local dictation" note="No account or API key required">
      <SettingsRow label="English speech model" description="English (en-US) · audio stays on this Mac"
        lead={<Mic size={14} strokeWidth={1.7} aria-hidden="true" />}
        control={status ? <StatusPill tone={tone(status.state)}>{STATE_LABEL[status.state]}</StatusPill> : <StatusPill>{loadError ? "Unavailable" : "Checking…"}</StatusPill>} />
      {loadError && <SettingsBlockRow>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p role="alert" className="text-xs text-destructive">{loadError}</p>
          <GhostButton disabled={busy} onClick={() => void refresh()}><RotateCcw size={12} aria-hidden="true" />Retry status</GhostButton>
        </div>
      </SettingsBlockRow>}
      {status?.reason && <SettingsBlockRow label={status.state === "failed" ? "Setup failed" : "Availability"}>
        <p role={status.state === "failed" ? "alert" : undefined} className={status.state === "failed" ? "text-xs leading-relaxed text-destructive" : "text-xs leading-relaxed text-muted-foreground"}>{status.reason}</p>
      </SettingsBlockRow>}
      {status && status.state !== "ready" && status.state !== "unsupported" && <SettingsBlockRow>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-xs text-muted-foreground">{bytes(status.downloadBytes)} download · about {bytes(status.installedBytes)} installed</p>
          <GhostButton disabled={busy || active} onClick={() => void mutate("setup")}>
            {status.state === "failed" ? <RotateCcw size={12} aria-hidden="true" /> : <Download size={12} aria-hidden="true" />}
            {busy ? "Starting…" : active ? STATE_LABEL[status.state] : status.state === "failed" ? "Retry download" : "Download and install"}
          </GhostButton>
        </div>
        <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">NVIDIA Open Model License. <a href="https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/" target="_blank" rel="noreferrer" className="underline underline-offset-2 hover:text-foreground">Review model terms</a> before installing.</p>
      </SettingsBlockRow>}
      {active && <SettingsBlockRow label={STATE_LABEL[status.state]} description={`${bytes(status.downloadedBytes)} of ${bytes(status.downloadBytes)}`}>
        <progress className="h-1.5 w-full overflow-hidden rounded-full bg-muted [&::-webkit-progress-bar]:bg-muted [&::-webkit-progress-value]:rounded-full [&::-webkit-progress-value]:bg-foreground [&::-moz-progress-bar]:bg-foreground"
          aria-label="Local dictation setup" value={progress} max={100} />
      </SettingsBlockRow>}
      {status?.state === "ready" && <VoiceMicrophoneTest onActiveChange={setTesting} />}
      {status && <SettingsRow label="Model details" description="Engine, storage and license" onOpen={() => setDetails(value => !value)} expanded={details} />}
      {details && status && <SettingsBlockRow>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-xs">
          <dt className="text-muted-foreground">Model</dt><dd className="text-foreground">NVIDIA Nemotron Speech Streaming</dd>
          <dt className="text-muted-foreground">Engine</dt><dd className="font-mono text-foreground">sherpa-onnx {status.engineVersion}</dd>
          <dt className="text-muted-foreground">Storage</dt><dd className="text-foreground">About {bytes(status.installedBytes)}</dd>
          <dt className="text-muted-foreground">License</dt><dd className="text-foreground">NVIDIA Open Model License</dd>
        </dl>
      </SettingsBlockRow>}
      {status?.state === "ready" && <SettingsBlockRow>
        <TextButton tone="destructive" disabled={busy || testing} onClick={() => void mutate("remove")}><Trash2 size={12} aria-hidden="true" />{busy ? "Removing…" : "Remove local model"}</TextButton>
      </SettingsBlockRow>}
    </SettingsGroup>
  </SettingsPage>;
}
