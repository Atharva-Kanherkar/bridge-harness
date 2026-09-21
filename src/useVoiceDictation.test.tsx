// @vitest-environment jsdom
import { StrictMode, act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { VoiceTranscriptPayload } from "./api";
import { useVoiceDictation } from "./useVoiceDictation";

const mocks = vi.hoisted(() => ({
  voiceCapabilities: vi.fn(), onVoiceTranscript: vi.fn(), voiceStart: vi.fn(),
  voiceAppend: vi.fn(), voiceStop: vi.fn(), voiceCancel: vi.fn(), capture: vi.fn(),
}));
vi.mock("./api", () => ({ bridgeApi: mocks }));
vi.mock("./voiceCapture", async original => ({
  ...await original<typeof import("./voiceCapture")>(), VoiceCapture: { start: mocks.capture },
}));

let root: Root;
let container: HTMLDivElement;
let voice: ReturnType<typeof useVoiceDictation>;
let listener: (event: VoiceTranscriptPayload) => void;
const off = vi.fn();
const captureStop = vi.fn();
const commit = vi.fn();
const available = { sessionId: "chat", providers: [{ provider: "codex", available: true }] };

function Harness({ harness = "codex", owner = "chat" }: { harness?: string; owner?: string }) {
  voice = useVoiceDictation({
    ownerKey: owner, harness, kind: "direct", runtimeStatus: "idle", working: false,
    readDraft: () => ({ ownerKey: owner, text: "draft", revision: 0, selectionStart: 5, selectionEnd: 5 }),
    commit,
  });
  return <span>{voice.state}</span>;
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  captureStop.mockResolvedValue(undefined);
  mocks.capture.mockResolvedValue({ stop: captureStop });
  mocks.voiceCapabilities.mockResolvedValue(available);
  mocks.onVoiceTranscript.mockImplementation(async handler => { listener = handler; return off; });
  mocks.voiceStart.mockResolvedValue({ sessionId: "chat", voiceSessionId: "take", provider: "codex",
    encoding: "pcm_s16_le", sampleRate: 16_000, channels: 1, maxChunkBytes: 65536, maxSessionBytes: 4194304 });
  mocks.voiceCancel.mockResolvedValue(undefined);
  container = document.createElement("div");
  document.body.append(container);
  act(() => { root = createRoot(container); });
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

it("refreshes capability on a harness change without requiring a different chat ID", async () => {
  await act(async () => root.render(<Harness />));
  expect(voice.available).toBe(true);
  mocks.voiceCapabilities.mockResolvedValue({ ...available,
    providers: [{ provider: "codex", available: false, unavailableReason: "Codex only" }] });
  await act(async () => root.render(<Harness harness="claude" />));
  expect(voice.available).toBe(false);
  expect(voice.unavailableReason).toBe("Codex only");
  expect(mocks.voiceCapabilities).toHaveBeenCalledTimes(2);
});

it("cleans StrictMode subscriptions and cancels capture when its owner changes", async () => {
  await act(async () => root.render(<StrictMode><Harness /></StrictMode>));
  expect(mocks.onVoiceTranscript).toHaveBeenCalledTimes(2);
  expect(off).toHaveBeenCalledTimes(1);
  await act(async () => { await voice.start(); });
  act(() => listener({ sessionId: "chat", voiceSessionId: "take", provider: "codex", kind: "started" }));
  expect(voice.state).toBe("recording");
  await act(async () => root.render(<StrictMode><Harness owner="other" /></StrictMode>));
  expect(captureStop).toHaveBeenCalledTimes(1);
  expect(mocks.voiceCancel).toHaveBeenCalledWith("take");
  act(() => listener({ sessionId: "chat", voiceSessionId: "take", provider: "codex", kind: "final", text: "late" }));
  expect(commit).not.toHaveBeenCalled();
});

it("releases a delayed subscription after the hook has unmounted", async () => {
  let resolve!: (off: () => void) => void;
  mocks.onVoiceTranscript.mockReturnValueOnce(new Promise<() => void>(yes => { resolve = yes; }));
  await act(async () => root.render(<Harness />));
  expect(voice.available).toBe(false);
  act(() => root.render(null));
  await act(async () => resolve(off));
  expect(off).toHaveBeenCalledTimes(1);
});

it("cancels a take when the document is hidden", async () => {
  await act(async () => root.render(<Harness />));
  await act(async () => { await voice.start(); });
  act(() => listener({ sessionId: "chat", voiceSessionId: "take", provider: "codex", kind: "started" }));
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  act(() => document.dispatchEvent(new Event("visibilitychange")));
  visibility.mockRestore();
  expect(captureStop).toHaveBeenCalledTimes(1);
  expect(mocks.voiceCancel).toHaveBeenCalledWith("take");
  expect(voice.state).toBe("idle");
});
