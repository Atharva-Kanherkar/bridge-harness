import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import { VoiceCapture } from "./voiceCapture";
import { VoiceDictationController, type VoiceDraft, type VoiceView } from "./voiceDictation";

type Options = {
  ownerKey?: string;
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
  const [capability, setCapability] = useState({ available: false, reason: "Checking dictation availability…" });
  const [listening, setListening] = useState(false);
  const [subscriptionError, setSubscriptionError] = useState<string>();
  const [refresh, setRefresh] = useState(0);
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
    setCapability({ available: false, reason: "Checking dictation availability…" });
    if (!options.ownerKey || options.working || options.kind !== "direct") {
      setCapability({ available: false, reason: options.working
        ? "Wait for the current turn to finish before dictating."
        : "Dictation currently requires a live direct chat. Local dictation setup is not installed yet." });
      return;
    }
    void bridgeApi.voiceCapabilities(options.ownerKey).then(result => {
      if (!current) return;
      const provider = result.providers.find(item => item.provider === "codex");
      setCapability({ available: provider?.available ?? false,
        reason: provider?.unavailableReason ?? "No dictation provider is ready." });
    }).catch(() => {
      if (current) setCapability({ available: false, reason: "Could not check dictation availability. Retry." });
    });
    return () => { current = false; };
  }, [options.ownerKey, options.harness, options.kind, options.runtimeStatus, options.working, refresh]);

  // Check every commit, including updates from shortcuts and restored drafts.
  useLayoutEffect(() => { controller.draftChanged(); });
  useLayoutEffect(() => () => { controller.cancel(); }, [controller, options.ownerKey, options.harness, options.kind, options.runtimeStatus, options.working]);

  return {
    ...view,
    available: capability.available && listening,
    unavailableReason: subscriptionError ?? (capability.available && !listening ? "Connecting to dictation events…" : capability.reason),
    active: view.state === "starting" || view.state === "recording" || view.state === "stopping",
    start: () => capability.available && listening ? controller.start("codex") : Promise.resolve(),
    stop: () => controller.stop(),
    cancel: () => controller.cancel(),
    isActive: () => controller.active,
    retry: () => { controller.cancel(); setListening(false); setRefresh(value => value + 1); },
  };
}
