/**
 * Where the Agents pane's own state lives: which agents are pinned, which rows
 * are open, which failures have been dismissed, and how wide the scope is.
 *
 * One key, `bridge.agents-pane`, and deliberately **not** keyed by chat. Pinned
 * means sticky — an agent you pinned in one chat is still in the tray when you
 * are reading another — so a per-chat record would defeat the feature it was
 * added for. The same reasoning covers the expanded and acknowledged sets: a
 * failure you dismissed stays dismissed whichever chat you open, because
 * dismissing it is a statement about the failure, not about the window.
 *
 * Every read is defensive. This is untrusted input from a previous version of
 * the app (or a half-written value from a crashed one), and a dock that refuses
 * to open because its preferences are malformed is a much worse outcome than a
 * dock that forgets which rows were open.
 */

import { useCallback, useState } from "react";
import type { AgentsScope } from "./components/agentsModel";

export type { AgentsScope };

export interface AgentsPaneSettings {
  /** Agent ids, as `agentsModel` mints them. A pinned id the model no longer
   *  knows is kept: unpinning is the only way to drop one, so a chat switch can
   *  never silently empty the tray. */
  pinned: string[];
  expanded: string[];
  acknowledged: string[];
  scope: AgentsScope;
}

export const AGENTS_PANE_STORAGE_KEY = "bridge.agents-pane";

export function defaultAgentsPaneSettings(): AgentsPaneSettings {
  return { pinned: [], expanded: [], acknowledged: [], scope: "this-chat" };
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string" && entry !== "") : [];
}

export function readAgentsPaneSettings(storage: Pick<Storage, "getItem"> = localStorage): AgentsPaneSettings {
  let raw: string | null = null;
  try {
    raw = storage.getItem(AGENTS_PANE_STORAGE_KEY);
  } catch {
    return defaultAgentsPaneSettings();
  }
  if (!raw) return defaultAgentsPaneSettings();
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return defaultAgentsPaneSettings();
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return defaultAgentsPaneSettings();
  const record = parsed as Record<string, unknown>;
  return {
    pinned: stringList(record.pinned),
    expanded: stringList(record.expanded),
    acknowledged: stringList(record.acknowledged),
    scope: record.scope === "all-chats" ? "all-chats" : "this-chat",
  };
}

export function writeAgentsPaneSettings(settings: AgentsPaneSettings, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(AGENTS_PANE_STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // A read-only storage must never stop the pane from working; it just forgets.
  }
}

/** Add or remove one id, preserving order and never duplicating. */
export function toggleAgentId(ids: readonly string[], id: string): string[] {
  return ids.includes(id) ? ids.filter(entry => entry !== id) : [...ids, id];
}

/** Add ids, idempotently. Acknowledging a failure is a statement, not a
 *  toggle: a second click on the same row must not un-dismiss it. */
export function addAgentIds(ids: readonly string[], added: readonly string[]): string[] {
  const next = [...ids];
  for (const id of added) if (!next.includes(id)) next.push(id);
  return next;
}

export function agentsPaneSettingsWith(settings: AgentsPaneSettings, patch: Partial<AgentsPaneSettings>): AgentsPaneSettings {
  return { ...settings, ...patch };
}

/**
 * The same record, live.
 *
 * Restored synchronously so the first paint already has the right rows open —
 * a pane that mounts collapsed and then springs open a second later reads as a
 * glitch, and the acknowledgement dim in particular has to be right immediately
 * or a dismissed failure flashes red on every reload.
 */
export function useAgentsPaneSettings(): [AgentsPaneSettings, (patch: Partial<AgentsPaneSettings> | ((previous: AgentsPaneSettings) => Partial<AgentsPaneSettings>)) => void] {
  const [settings, setSettings] = useState<AgentsPaneSettings>(() => readAgentsPaneSettings());
  const update = useCallback((patch: Partial<AgentsPaneSettings> | ((previous: AgentsPaneSettings) => Partial<AgentsPaneSettings>)) => {
    setSettings(previous => {
      const next = agentsPaneSettingsWith(previous, typeof patch === "function" ? patch(previous) : patch);
      writeAgentsPaneSettings(next);
      return next;
    });
  }, []);
  return [settings, update];
}
