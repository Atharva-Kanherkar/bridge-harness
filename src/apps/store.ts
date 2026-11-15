// The apps as core reports them: which are on, where each is pinned, and what
// each is waiting on. One module-level store, so the launcher, the sidebar
// and the Store read the same list and redraw together.
//
// Core owns the truth. This store reads `apps/list_apps`, rereads on
// `apps/changed` (a burst collapses into one read every 250 ms), and keeps
// only the newest answer. Until the first answer arrives every built-in reads
// as on, unpinned and quiet, which is what a fresh install looks like.

import { useSyncExternalStore } from "react";
import { bridgeApi } from "../api";
import type { AppSummary, ListAppsResult } from "../protocol/generated/protocol";
import { BUILTIN_APPS, isAppId, type AppBadges, type AppId, type BuiltinApp } from "./registry";

export const COALESCE_MS = 250;
/** Where slice 1 kept pins in this browser. Read once for the import. */
export const PINS_KEY = "bridge.apps.pins.v1";
export const MAX_PINS = 3;

type Snapshot = { apps: readonly AppSummary[] | null };

const EMPTY: Snapshot = { apps: null };
let snapshot: Snapshot = EMPTY;
const listeners = new Set<() => void>();
let started = false;
let unlisten: (() => void) | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
let requested = 0;
let applied = 0;
const firstLoadHooks: Array<() => void> = [];

function notify() {
  for (const listener of [...listeners]) listener();
}

/** Keep `result` unless a newer request already answered. */
function apply(result: ListAppsResult, sequence: number) {
  if (sequence < applied) return;
  applied = sequence;
  const first = snapshot.apps === null;
  snapshot = { apps: result.apps };
  notify();
  if (first) for (const hook of firstLoadHooks.splice(0)) hook();
}

/** Read the list now. Errors keep the last list; the next hint retries. */
export function loadApps(): Promise<void> {
  const sequence = ++requested;
  return bridgeApi.listApps().then(result => apply(result, sequence), () => undefined);
}

/** Read the list once the current burst of hints settles. */
export function scheduleLoad() {
  if (timer !== null) return;
  timer = setTimeout(() => {
    timer = null;
    void loadApps();
  }, COALESCE_MS);
}

function start() {
  if (started) return;
  started = true;
  afterFirstLoad(() => { void importLegacyPins(); });
  void loadApps();
  void bridgeApi.onAppsChanged(scheduleLoad).then(off => {
    if (!started) { off(); return; }
    unlisten = off;
  }).catch(() => undefined);
}

export function subscribeApps(listener: () => void): () => void {
  start();
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function read(): Snapshot {
  return snapshot;
}

/** Every app core lists, or null before the first answer. */
export function useApps(): readonly AppSummary[] | null {
  return useSyncExternalStore(subscribeApps, read, read).apps;
}

/** Run `hook` once, after the first list arrives. */
export function afterFirstLoad(hook: () => void) {
  if (snapshot.apps !== null) hook();
  else firstLoadHooks.push(hook);
}

/** A setter's answer is the new list; apply it at once. */
export function applyAnswer(result: ListAppsResult) {
  apply(result, ++requested);
}

// Slice 1 pins ---------------------------------------------------------------

/** The ids slice 1 stored: strings only, repeats dropped, at most three. */
export function parsePins(raw: string | null): readonly string[] {
  if (!raw) return [];
  let stored: unknown;
  try {
    stored = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(stored)) return [];
  const pins: string[] = [];
  for (const value of stored) {
    if (pins.length === MAX_PINS) break;
    if (typeof value === "string" && !pins.includes(value)) pins.push(value);
  }
  return pins;
}

/** Move slice 1's local pins into core, in order, once. A full sidebar or an
 *  app core refuses only skips that pin; the key goes away first, so the
 *  import never runs twice. */
export async function importLegacyPins(): Promise<void> {
  let raw: string | null = null;
  try {
    if (typeof localStorage === "undefined") return;
    raw = localStorage.getItem(PINS_KEY);
    if (raw === null) return;
    localStorage.removeItem(PINS_KEY);
  } catch {
    return;
  }
  for (const id of parsePins(raw)) {
    try {
      applyAnswer(await bridgeApi.setAppPinned(id, true));
    } catch {
      // Unknown, off, or the sidebar is full: skip this one.
    }
  }
}

// Views over the list -------------------------------------------------------

/** Built-in apps with a screen that are on, in launcher order. Before the
 *  first answer, all of them. */
export function enabledApps(apps: readonly AppSummary[] | null): readonly BuiltinApp[] {
  if (!apps) return BUILTIN_APPS;
  const on = new Set(apps.filter(app => app.enabled).map(app => app.manifest.id));
  return BUILTIN_APPS.filter(app => on.has(app.id));
}

/** Pinned apps that have a screen, in sidebar order. */
export function pinsOf(apps: readonly AppSummary[] | null): readonly AppId[] {
  if (!apps) return [];
  return apps
    .filter(app => app.enabled && app.pinned !== null && app.pinned !== undefined && isAppId(app.manifest.id))
    .sort((a, b) => (a.pinned ?? 0) - (b.pinned ?? 0))
    .map(app => app.manifest.id as AppId);
}

/** Each badge core reported, by app id. */
export function badgesOf(apps: readonly AppSummary[] | null): AppBadges {
  const badges: AppBadges = {};
  for (const app of apps ?? []) {
    if (app.badge && isAppId(app.manifest.id)) badges[app.manifest.id] = { count: app.badge.count };
  }
  return badges;
}

/** Tests only. */
export function resetAppsStore() {
  if (timer !== null) clearTimeout(timer);
  timer = null;
  unlisten?.();
  unlisten = null;
  started = false;
  snapshot = EMPTY;
  requested = 0;
  applied = 0;
  firstLoadHooks.length = 0;
  listeners.clear();
}

/** Tests only: the list as the store holds it. */
export function __snapshotForTests(): readonly AppSummary[] | null {
  return snapshot.apps;
}
