// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import { FIRST_PAGE_ENTRIES } from "./forestCache";
import { PREFETCH_INTENT_MS } from "./components/BridgeSidebar";
import type { BridgeState, Session, SessionEntry, SessionForestSnapshot } from "./types";

let container: HTMLDivElement;
let root: Root;
const chat = (id: string): Session => ({
  id, harness: "claude", kind: "chat", label: id, title: id,
  status: "ready", metricSource: "provider", restorationMode: "fresh",
  continuationFidelity: "native",
});

/** Entries `from..=to` of a single-chain chat whose active branch is `total` long. */
function forest(sessionId: string, from: number, to: number, total: number): SessionForestSnapshot {
  const entries: SessionEntry[] = [];
  for (let sequence = from; sequence <= to; sequence += 1) {
    entries.push({
      id: `${sessionId}-${sequence}`, sessionId, parentEntryId: sequence > 1 ? `${sessionId}-${sequence - 1}` : null,
      sequence, semanticSchemaVersion: 1, kind: sequence % 2 ? "user.message" : "assistant.message",
      payload: { text: `${sessionId} message ${sequence}` }, providerEventId: null,
      contextVisibility: "eligible", tokenEstimate: null, createdAt: "2026-11-14T00:00:00Z",
    } as SessionEntry);
  }
  const last = entries[entries.length - 1];
  return {
    sessionId, entries, activeProcessEntries: [],
    head: { sessionId, activeEntryId: last.id, nativeProviderSessionId: null, restorationMode: "fresh", resumeEligibility: "fresh", latestCheckpointEntryId: null, updatedAt: "now" },
    leaves: [last], workerLeases: [], workerRuntimes: [], workerQueue: [], usage: [], reasons: [],
    policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1, maxCapabilityUnitsPerTurn: 24 },
    repositoryDivergence: { status: "unknown", selectedState: null, currentState: { status: "unavailable" } },
    completion: null,
    entryWindow: { returned: entries.length, total, trimmedPayloads: 0 },
  } as SessionForestSnapshot;
}

/** A 2,000-event chat: the daemon pages its newest entries, and its full
 * window is the newest 900. */
const TOTAL = 2000;
const FULL_FROM = TOTAL - 900 + 1;
let releaseFull: Array<() => void>;
let forestCalls: Array<[string, number | undefined]>;

async function settle(ms = 20) {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });
}

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => store.set(key, value),
      removeItem: (key: string) => store.delete(key), clear: () => store.clear(),
    },
  });
  const state: BridgeState = { projects: [], workspaces: [], sessions: [chat("Alpha"), chat("Beta")], events: [] };
  vi.spyOn(bridgeApi, "state").mockImplementation(async () => state);
  vi.spyOn(bridgeApi, "sessionForestDigest").mockImplementation(async id => `digest-${id}`);
  vi.spyOn(bridgeApi, "pendingWorkerAdoptions").mockResolvedValue([]);
  releaseFull = [];
  forestCalls = [];
  vi.spyOn(bridgeApi, "sessionForest").mockImplementation((id, options = {}) => {
    forestCalls.push([id, options.limit]);
    if (options.limit !== undefined) return Promise.resolve(forest(id, TOTAL - options.limit + 1, TOTAL, TOTAL));
    // The full window is held until the test lets it go.
    return new Promise(resolve => { releaseFull.push(() => resolve(forest(id, FULL_FROM, TOTAL, TOTAL))); });
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
  const recommended = [...container.querySelectorAll("button")].find(button => button.textContent === "Use recommended defaults");
  if (recommended) { await act(async () => recommended.click()); await settle(); }
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const row = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button[title]")].find(button => button.title.startsWith(`${name} —`));
const windowNotice = () => [...container.querySelectorAll('[role="status"]')].find(node => node.textContent?.includes("Showing the most recent"));

it("paints a long chat's newest page first, then fetches its full window right behind it", async () => {
  expect(row("Alpha")).toBeTruthy();
  await act(async () => row("Alpha")!.click());
  await settle();
  // The page went first; the full window is already on its way, not left to
  // the next poll three seconds later.
  expect(forestCalls.filter(([id]) => id === "Alpha")).toEqual([["Alpha", FIRST_PAGE_ENTRIES], ["Alpha", undefined]]);
  expect(container.textContent).toContain(`Alpha message ${TOTAL}`);
  // A first page is not a window onto the chat: no "most recent 160" notice.
  expect(windowNotice()).toBeUndefined();

  await act(async () => releaseFull.forEach(release => release()));
  await settle();
  expect(container.textContent).toContain(`Alpha message ${TOTAL}`);
  expect(windowNotice()?.textContent).toContain(`Showing the most recent 900 of ${TOTAL.toLocaleString()} events`);
});

it("reopens an unchanged chat from the cache without reading its snapshot again", async () => {
  await act(async () => row("Alpha")!.click());
  await settle();
  await act(async () => releaseFull.forEach(release => release()));
  await settle();
  // Somewhere else, then back.
  await act(async () => row("Beta")!.click());
  await settle();
  const before = forestCalls.filter(([id]) => id === "Alpha").length;
  await act(async () => row("Alpha")!.click());
  // Painted on the selection commit, before any read resolves.
  expect(container.textContent).toContain(`Alpha message ${TOTAL}`);
  await settle();
  // The digest matched the cached one, so no snapshot was read at all.
  expect(forestCalls.filter(([id]) => id === "Alpha").length).toBe(before);
  expect(bridgeApi.pendingWorkerAdoptions).toHaveBeenCalledWith("Alpha");
});

it("warms a chat the pointer rests on, so the click paints from memory", async () => {
  await act(async () => row("Alpha")!.click());
  await settle();
  const beta = row("Beta")!;
  // A pointer passing straight over the row reads nothing.
  await act(async () => { beta.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })); beta.dispatchEvent(new MouseEvent("pointerout", { bubbles: true })); });
  await settle(PREFETCH_INTENT_MS + 30);
  expect(forestCalls.some(([id]) => id === "Beta")).toBe(false);

  // A resting pointer reads the newest page.
  await act(async () => { beta.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })); });
  await settle(PREFETCH_INTENT_MS + 30);
  expect(forestCalls.filter(([id]) => id === "Beta")).toEqual([["Beta", FIRST_PAGE_ENTRIES]]);

  await act(async () => beta.click());
  // On screen at once, from the warmed page.
  expect(container.textContent).toContain(`Beta message ${TOTAL}`);
  await settle();
  // No second page: the full window is all that is left to read.
  expect(forestCalls.filter(([id]) => id === "Beta")).toEqual([["Beta", FIRST_PAGE_ENTRIES], ["Beta", undefined]]);
});
