// Transcript display preferences. One switch today: whether the model's
// reasoning text is drawn at all. Stored locally, read by the transcript and
// by the Appearance page, and broadcast so flipping the switch applies to
// every open conversation without a reload.

import { useEffect, useState } from "react";

export const SHOW_THINKING_STORAGE_KEY = "bridge.transcript.showThinking";

const SHOW_THINKING_EVENT = "bridge:transcript-settings";

export function readShowThinking(storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(SHOW_THINKING_STORAGE_KEY) !== "false";
  } catch {
    return true;
  }
}

export function writeShowThinking(value: boolean, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(SHOW_THINKING_STORAGE_KEY, value ? "true" : "false");
  } catch {
    // A read-only storage should never stop the transcript from rendering.
  }
}

/** Reads the stored preference and stays in sync with writes from any other mounted consumer. */
export function useShowThinking(): [boolean, (next: boolean) => void] {
  const [value, setValue] = useState<boolean>(() => readShowThinking());

  useEffect(() => {
    const onExternalChange = () => setValue(readShowThinking());
    window.addEventListener(SHOW_THINKING_EVENT, onExternalChange);
    window.addEventListener("storage", onExternalChange);
    return () => {
      window.removeEventListener(SHOW_THINKING_EVENT, onExternalChange);
      window.removeEventListener("storage", onExternalChange);
    };
  }, []);

  const setShowThinking = (next: boolean) => {
    writeShowThinking(next);
    setValue(next);
    window.dispatchEvent(new Event(SHOW_THINKING_EVENT));
  };

  return [value, setShowThinking];
}
