// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { LAST_DRAFT_MODEL_KEY, readLastDraftModel, writeLastDraftModel } from "./lastDraftModel";

let original: PropertyDescriptor | undefined;
const store = new Map<string, string>();

beforeEach(() => {
  store.clear();
  original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
    },
  });
});

afterEach(() => {
  if (original) Object.defineProperty(globalThis, "localStorage", original);
  else Reflect.deleteProperty(globalThis, "localStorage");
});

describe("lastDraftModel", () => {
  it("TestReadLastDraftModel_RoundTrips", () => {
    writeLastDraftModel({ harness: "opencode", model: "opencode-go/kimi", label: "Kimi K2.5" });
    expect(readLastDraftModel()).toEqual({ harness: "opencode", model: "opencode-go/kimi", label: "Kimi K2.5" });
  });

  it("TestReadLastDraftModel_IgnoresMalformedPayload", () => {
    store.set(LAST_DRAFT_MODEL_KEY, "{not json");
    expect(readLastDraftModel()).toBeNull();
    store.set(LAST_DRAFT_MODEL_KEY, JSON.stringify({ harness: "", model: null }));
    expect(readLastDraftModel()).toBeNull();
    store.set(LAST_DRAFT_MODEL_KEY, JSON.stringify({ harness: "codex", model: 7 }));
    expect(readLastDraftModel()).toBeNull();
    store.set(LAST_DRAFT_MODEL_KEY, JSON.stringify(null));
    expect(readLastDraftModel()).toBeNull();
  });

  it("TestReadLastDraftModel_KeepsUnknownHarnessForTheResolver", () => {
    // Shape only: an id that no adapter reports is still returned, and the
    // resolver is the one that drops it. This keeps the read honest about
    // what it knows.
    writeLastDraftModel({ harness: "gone", model: "m" });
    expect(readLastDraftModel()).toEqual({ harness: "gone", model: "m", label: null });
  });

  it("TestWriteLastDraftModel_NoStorageDoesNotThrow", () => {
    Reflect.deleteProperty(globalThis, "localStorage");
    expect(() => writeLastDraftModel({ harness: "codex", model: null })).not.toThrow();
    expect(readLastDraftModel()).toBeNull();
  });
});
