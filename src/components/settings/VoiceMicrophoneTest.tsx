import { useEffect, useId, useRef, useState } from "react";
import { useVoiceDictation } from "../../useVoiceDictation";
import { VoiceControls, VoiceStatus } from "../VoiceControls";
import { SettingsBlockRow, TextButton } from "./kit";

/** Uses production capture and transcription, without creating or sending a chat. */
export function VoiceMicrophoneTest({ onActiveChange }: { onActiveChange(active: boolean): void }) {
  const ownerKey = `voice-test:${useId()}`;
  const [text, setText] = useState("");
  const draft = useRef({ text: "", revision: 0 });
  const input = useRef<HTMLTextAreaElement>(null);
  const container = useRef<HTMLDivElement>(null);
  const update = (value: string) => {
    draft.current = { text: value, revision: draft.current.revision + 1 };
    setText(value);
  };
  const voice = useVoiceDictation({ ownerKey, provider: "local",
    readDraft: () => ({ ...draft.current, ownerKey,
      selectionStart: input.current?.selectionStart ?? draft.current.text.length,
      selectionEnd: input.current?.selectionEnd ?? draft.current.text.length }),
    commit: (value, caret) => {
      update(value);
      const revision = draft.current.revision;
      requestAnimationFrame(() => {
        if (draft.current.revision === revision && input.current?.value === value) { input.current.focus({ preventScroll: true }); input.current.setSelectionRange(caret, caret); }
      });
    },
  });
  const notify = useRef(onActiveChange);
  notify.current = onActiveChange;
  useEffect(() => { notify.current(voice.active); }, [voice.active]);
  useEffect(() => () => { notify.current(false); }, []);
  const previousState = useRef(voice.state);
  useEffect(() => {
    if (previousState.current === voice.state) return;
    previousState.current = voice.state;
    const focused = document.activeElement;
    // Pointer presses prevent focus, while keyboard holds must keep button focus.
    // Disabling the stop button can also blur it during finalization.
    // Restore the transcript without stealing unrelated settings focus.
    if (focused === document.body || (!voice.active && container.current?.contains(focused) && focused?.hasAttribute("data-voice-control"))) {
      input.current?.focus({ preventScroll: true });
    }
  }, [voice.active, voice.state]);

  return <SettingsBlockRow label="Try your microphone" description="Say a short sentence, then finish to review it here.">
    <div ref={container} className="u-glass-soft rounded-lg border border-border px-3 py-2"
      onKeyDownCapture={event => {
        if (!voice.active || event.nativeEvent.isComposing) return;
        if (event.key === "Escape" || (event.key === "Enter" && !(event.target instanceof Element && event.target.closest("button")))) {
          event.preventDefault();
          event.stopPropagation();
          if (event.key === "Escape") voice.cancel();
          else if (voice.state !== "stopping") void voice.stop();
        }
      }}>
      <textarea ref={input} aria-label="Microphone test transcript" rows={2} readOnly={voice.active}
        value={voice.active ? voice.draftPreview ?? text : text} placeholder="Your words will appear here…"
        onChange={event => update(event.target.value)}
        className="block min-h-14 w-full resize-none bg-transparent text-sm leading-relaxed text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring" />
      <VoiceStatus {...voice} />
      <div className="mt-1 flex items-center justify-between gap-2">
        <span className="text-[11px] text-muted-foreground">Audio stays on this Mac.</span>
        <div className="flex items-center gap-1">
          {text && !voice.active && <TextButton onClick={() => update("")}>Clear test</TextButton>}
          <VoiceControls state={voice.state} available={voice.available} unavailableReason={voice.unavailableReason}
            providerLabel="Test microphone · local English dictation" error={voice.error} recovery={voice.recovery}
            onStart={() => void voice.start()} onStop={() => void voice.stop()} onCancel={voice.cancel} onRetry={voice.retry} />
        </div>
      </div>
    </div>
  </SettingsBlockRow>;
}
