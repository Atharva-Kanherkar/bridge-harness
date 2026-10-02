// Whether Mission Control auto-surfaces worker chats (sessions with a
// parentSessionId) alongside the orchestrator that spawned them. Off by
// default: a busy orchestrator can spawn many workers at once, and showing
// every one of them makes it hard to tell which tile is the chat the user is
// actually steering. Explicitly pinning a worker tile always overrides this.

import { useEffect, useState } from "react";

export const SHOW_WORKER_CHATS_STORAGE_KEY = "bridge.missionControl.showWorkerChats";

const SHOW_WORKER_CHATS_EVENT = "bridge:mission-control-settings";

export function readShowWorkerChatsInMissionControl(storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(SHOW_WORKER_CHATS_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeShowWorkerChatsInMissionControl(
  value: boolean,
  storage: Pick<Storage, "setItem"> = localStorage,
): void {
  try {
    storage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, value ? "true" : "false");
  } catch {
    // A read-only storage should never stop Mission Control from rendering.
  }
}

/** Reads the stored preference and stays in sync with writes from any other mounted consumer. */
export function useShowWorkerChatsInMissionControl(): [boolean, (next: boolean) => void] {
  const [value, setValue] = useState<boolean>(() => readShowWorkerChatsInMissionControl());

  useEffect(() => {
    const onExternalChange = () => setValue(readShowWorkerChatsInMissionControl());
    window.addEventListener(SHOW_WORKER_CHATS_EVENT, onExternalChange);
    window.addEventListener("storage", onExternalChange);
    return () => {
      window.removeEventListener(SHOW_WORKER_CHATS_EVENT, onExternalChange);
      window.removeEventListener("storage", onExternalChange);
    };
  }, []);

  const setShowWorkerChats = (next: boolean) => {
    writeShowWorkerChatsInMissionControl(next);
    setValue(next);
    window.dispatchEvent(new Event(SHOW_WORKER_CHATS_EVENT));
  };

  return [value, setShowWorkerChats];
}
