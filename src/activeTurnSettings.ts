// "Input while running": what a message sent during a live turn does.
//
// One source: the chat's own configuration once it has one (revision above
// 0), otherwise the application chat default saved on Settings, New chats,
// otherwise Steer. There used to be a second toggle on General that wrote
// localStorage; configured chats ignored it, so it looked like it worked and
// did nothing. Its last value is migrated into the application default once.

import { useEffect, useSyncExternalStore } from "react";
import { bridgeApi } from "./api";
import type { ActiveTurnInput, ChatDefaultsView, ChatSessionSettings } from "./protocol/generated/protocol";

export type { ActiveTurnInput };

/** The retired General toggle's key, read only by the one-time migration. */
export const LEGACY_ACTIVE_TURN_INPUT_KEY = "bridge.composer.activeTurnInput";

let applicationDefault: ActiveTurnInput | null = null;
let loading: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(next: ActiveTurnInput | null) {
  if (next === applicationDefault) return;
  applicationDefault = next;
  for (const listener of listeners) listener();
}

/** The chat's own value when it has a configuration, else the application default, else Steer. */
export function resolveActiveTurnInput(session: ChatSessionSettings | undefined, fallback: ActiveTurnInput | null): ActiveTurnInput {
  if (session && session.revision > 0) return session.settings.activeTurnInput;
  return fallback ?? "steer";
}

/**
 * Moves a legacy "queue" into the application default when that default is
 * unset, then forgets the legacy key. A set default always wins. A failed save
 * keeps the key, so the next launch tries again.
 */
export async function migrateLegacyActiveTurnInput(view: ChatDefaultsView, storage: Pick<Storage, "getItem" | "removeItem"> = localStorage): Promise<ChatDefaultsView> {
  let legacy: string | null;
  try { legacy = storage.getItem(LEGACY_ACTIVE_TURN_INPUT_KEY); } catch { return view; }
  if (legacy === null) return view;
  let next = view;
  if (legacy === "queue" && view.application.defaults.activeTurnInput == null) {
    try {
      next = await bridgeApi.saveChatDefaults({
        projectId: null,
        expectedRevision: view.application.revision,
        defaults: { ...view.application.defaults, activeTurnInput: "queue" },
      });
    } catch { return view; }
  }
  try { storage.removeItem(LEGACY_ACTIVE_TURN_INPUT_KEY); } catch { /* read-only storage: harmless, the default is set */ }
  return next;
}

/** Take a freshly loaded or saved application view as the new truth. */
export function acceptApplicationChatDefaults(view: Pick<ChatDefaultsView, "application">): void {
  emit(view.application.defaults.activeTurnInput ?? null);
}

/** Reload the application default from the host, migrating the legacy value first. */
export function refreshApplicationActiveTurnInput(): Promise<void> {
  loading ??= bridgeApi.getChatDefaults(null)
    .then(view => migrateLegacyActiveTurnInput(view))
    .then(acceptApplicationChatDefaults)
    .catch(() => undefined)
    .finally(() => { loading = null; });
  return loading;
}

let loadedOnce = false;
function subscribe(onChange: () => void): () => void {
  listeners.add(onChange);
  return () => { listeners.delete(onChange); };
}

/** Test seam: forget what was loaded. */
export function resetActiveTurnInputForTests(): void {
  applicationDefault = null;
  loadedOnce = false;
  loading = null;
}

/** The resolved value for one chat. Pass the chat's configuration when known. */
export function useActiveTurnInput(session?: ChatSessionSettings): ActiveTurnInput {
  const fallback = useSyncExternalStore(subscribe, () => applicationDefault, () => null);
  useEffect(() => {
    if (loadedOnce) return;
    loadedOnce = true;
    void refreshApplicationActiveTurnInput();
  }, []);
  return resolveActiveTurnInput(session, fallback);
}
