import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import type { VoiceProviderId } from "./protocol/generated/protocol";
import { VoiceCapture } from "./voiceCapture";
import { VoiceDictationController, type VoiceDraft, type VoiceView } from "./voiceDictation";

type Options = {
  ownerKey?: string;
  sessionId?: string;
  provider: VoiceProviderId;
  harness?: string;
  kind?: string;
  runtimeStatus?: string;
  working: boolean;
  readDraft(): VoiceDraft;
  commit(text: string, caret: number): void;
};

/** React owns presentation; the controller owns all async recording resources. */
export function useVoiceDictation(options: Options) {
  const latest = useRef(options);
  latest.current = options;
  const [view, setView] = useState<VoiceView>({ state: "idle", preview: "" });
  const [capability, setCapability] = useState({ scope: "", available: false, reason: "Checking dictation availability…" });
  const [listening, setListening] = useState(false);
  const [subscriptionError, setSubscriptionError] = useState<string>();
  const [refresh, setRefresh] = useState(0);
  const scope = JSON.stringify([options.ownerKey, options.sessionId, options.provider, options.harness, options.kind, options.runtimeStatus, options.working, refresh]);
  const [controller] = useState(() => new VoiceDictationController({
    transport: {
      start: bridgeApi.voiceStart,
      append: bridgeApi.voiceAppend,
      stop: bridgeApi.voiceStop,
      cancel: bridgeApi.voiceCancel,
    },
    capture: (chunk, error) => VoiceCapture.start(chunk, error),
    readDraft: () => latest.current.readDraft(),
    commit: (text, caret) => latest.current.commit(text, caret),
    changed: setView,
  }));

  useEffect(() => {
    let active = true;
    let off: (() => void) | undefined;
    setListening(false);
    setSubscriptionError(undefined);
    void bridgeApi.onVoiceTranscript(event => { if (active) controller.receive(event); }).then(unlisten => {
      if (active) { off = unlisten; setListening(true); } else unlisten();
    }).catch(() => {
      if (active) setSubscriptionError("Could not connect to dictation events. Retry.");
    });
    return () => { active = false; off?.(); controller.cancel(); };
  }, [controller, refresh]);

  useEffect(() => {
    const onHidden = () => { if (document.visibilityState === "hidden") controller.cancel(); };
    const onPageHide = () => controller.cancel();
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pagehide", onPageHide);
    return () => {
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pagehide", onPageHide);
    };
  }, [controller]);

  useEffect(() => {
    let current = true;
    setCapability({ scope, available: false, reason: "Checking dictation availability…" });
    if (options.working) {
      setCapability({ scope, available: false, reason: "Wait for the current turn to finish before dictating." });
      return;
    }
    void bridgeApi.voiceCapabilities(options.sessionId).then(result => {
      if (!current) return;
      const provider = result.providers.find(item => item.provider === options.provider);
      setCapability({ scope, available: !!options.ownerKey && provider?.state === "ready",
        reason: provider?.unavailableReason ?? "No dictation provider is ready." });
    }).catch(() => {
      if (current) setCapability({ scope, available: false, reason: "Could not check dictation availability. Retry." });
    });
    return () => { current = false; };
  }, [scope, options.ownerKey, options.sessionId, options.provider, options.harness, options.kind, options.runtimeStatus, options.working, refresh]);

  // Check every commit, including updates from shortcuts and restored drafts.
  useLayoutEffect(() => { controller.draftChanged(); });
  useLayoutEffect(() => () => { controller.cancel(); }, [controller, options.ownerKey, options.sessionId, options.provider, options.harness, options.kind, options.runtimeStatus, options.working]);

  const currentCapability = capability.scope === scope;
  const available = currentCapability && capability.available && listening;

  return {
    ...view,
    available,
    unavailableReason: subscriptionError ?? (!currentCapability ? "Checking dictation availability…" : capability.available && !listening ? "Connecting to dictation events…" : capability.reason),
    active: view.state === "starting" || view.state === "recording" || view.state === "stopping",
    start: () => available ? controller.start(options.provider) : Promise.resolve(),
    stop: () => controller.stop(),
    cancel: () => controller.cancel(),
    isActive: () => controller.active,
    retry: () => { controller.cancel(); setListening(false); setRefresh(value => value + 1); },
  };
}
