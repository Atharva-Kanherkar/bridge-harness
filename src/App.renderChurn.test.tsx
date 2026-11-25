// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

// Counts renders of the open chat's transcript. The stand-in keeps the real
// component's `memo` boundary, so a render here means App handed it a prop
// with a new identity.
const transcript = vi.hoisted(() => ({ renders: 0 }));

vi.mock("./components/AgentConversation", async importOriginal => {
  const actual = await importOriginal<typeof import("./components/AgentConversation")>();
  const { memo } = await import("react");
  const Counted = memo(function Counted(props: { leading?: unknown }) {
    // Only the main chat passes `leading`; Mission Control tiles and dock
    // agents render the same component without it.
    if (props.leading !== undefined) transcript.renders += 1;
    return null;
  });
  return { ...actual, AgentConversation: Counted };
});

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    options: Record<string, unknown> = {};
    open() {}
    loadAddon() {}
    dispose() {}
    onData() { return { dispose() {} }; }
    write() {}
    writeln() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  // jsdom here has no localStorage, and the sidebar reads it during mount.
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    },
  });
  HTMLElement.prototype.scrollIntoView = vi.fn();
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container);
    root.render(<App />);
  });
  await pause(20);
  const recommended = [...container.querySelectorAll("button")].find(button => button.textContent === "Use recommended defaults");
  if (recommended) {
    await act(async () => recommended.click());
    await pause(40);
  }
});

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  vi.restoreAllMocks();
});

const pause = (ms: number) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });

async function openChat() {
  const row = container.querySelector<HTMLButtonElement>('button[data-tip*=" — "]');
  expect(row).toBeTruthy();
  await act(async () => row!.click());
  await pause(80);
  const field = [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
    .find(candidate => candidate.placeholder.startsWith("Message Bridge") || candidate.placeholder.startsWith("Send a follow-up"));
  expect(field).toBeTruthy();
  return field!;
}

async function type(field: HTMLTextAreaElement, text: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setValue.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("typing in the composer", () => {
  it("does not re-render the open chat's transcript", async () => {
    const field = await openChat();
    expect(transcript.renders, "the open chat rendered its transcript").toBeGreaterThan(0);
    const before = transcript.renders;

    for (const text of ["h", "he", "hel", "hell", "hello"]) await type(field, text);
    await pause(20);

    expect(field.value).toBe("hello");
    expect(transcript.renders - before, "keystrokes re-rendered the transcript").toBe(0);
  });
});
