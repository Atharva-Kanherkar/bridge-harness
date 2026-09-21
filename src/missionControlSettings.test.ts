import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  readShowWorkerChatsInMissionControl,
  SHOW_WORKER_CHATS_STORAGE_KEY,
  writeShowWorkerChatsInMissionControl,
} from "./missionControlSettings";

let store: Map<string, string>;
beforeEach(() => {
  store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => store.set(key, value),
    removeItem: (key: string) => store.delete(key),
  });
});
afterEach(() => vi.unstubAllGlobals());

it("hides worker chats by default when nothing is stored", () => {
  expect(readShowWorkerChatsInMissionControl()).toBe(false);
});

it("round-trips an explicit write", () => {
  writeShowWorkerChatsInMissionControl(true);
  expect(store.get(SHOW_WORKER_CHATS_STORAGE_KEY)).toBe("true");
  expect(readShowWorkerChatsInMissionControl()).toBe(true);

  writeShowWorkerChatsInMissionControl(false);
  expect(readShowWorkerChatsInMissionControl()).toBe(false);
});

it("fails closed on a malformed stored value", () => {
  store.set(SHOW_WORKER_CHATS_STORAGE_KEY, "not-a-boolean");
  expect(readShowWorkerChatsInMissionControl()).toBe(false);
});

it("fails closed when storage throws", () => {
  const throwing: Pick<Storage, "getItem"> = {
    getItem: () => { throw new Error("denied"); },
  };
  expect(readShowWorkerChatsInMissionControl(throwing)).toBe(false);
});

it("does not throw when storage is read-only", () => {
  const throwing: Pick<Storage, "setItem"> = {
    setItem: () => { throw new Error("denied"); },
  };
  expect(() => writeShowWorkerChatsInMissionControl(true, throwing)).not.toThrow();
});
