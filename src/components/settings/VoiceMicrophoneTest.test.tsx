// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { VoiceTranscriptPayload } from "../../api";
import { VoiceMicrophoneTest } from "./VoiceMicrophoneTest";

const mocks = vi.hoisted(() => ({
  voiceCapabilities: vi.fn(), onVoiceTranscript: vi.fn(), voiceStart: vi.fn(),
  voiceAppend: vi.fn(), voiceStop: vi.fn(), voiceCancel: vi.fn(), capture: vi.fn(),
}));
vi.mock("../../api", () => ({ bridgeApi: mocks }));
vi.mock("../../voiceCapture", async original => ({
  ...await original<typeof import("../../voiceCapture")>(), VoiceCapture: { start: mocks.capture },
}));
let host: HTMLDivElement;
let root: Root;
let listener: (event: VoiceTranscriptPayload) => void;
let take: { ownerKey: string; provider: "local"; voiceSessionId: string };
const release = vi.fn();
const active = vi.fn();
function input() { return host.querySelector<HTMLTextAreaElement>("textarea")!; }
function button(label: string) { return host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!; }
function event(kind: VoiceTranscriptPayload["kind"], text?: string) { act(() => listener({ ...take, kind, text })); }

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { callback(0); return 1; });
  release.mockResolvedValue(undefined);
  mocks.capture.mockResolvedValue({ stop: release });
  mocks.voiceCapabilities.mockResolvedValue({ providers: [{ provider: "local", state: "ready" }] });
  mocks.onVoiceTranscript.mockImplementation(async handler => { listener = handler; return () => {}; });
  mocks.voiceStart.mockImplementation(async ownerKey => {
    take = { ownerKey, provider: "local", voiceSessionId: "microphone-test" };
    return { ...take, encoding: "pcm_s16_le", sampleRate: 16000, channels: 1, maxChunkBytes: 65536, maxSessionBytes: 4194304 };
  });
  mocks.voiceStop.mockResolvedValue(undefined);
  mocks.voiceCancel.mockResolvedValue(undefined);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root.render(<VoiceMicrophoneTest onActiveChange={active} />));
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it("uses local capture, previews without sending, finalizes, and can clear the test", async () => {
  expect(mocks.capture).not.toHaveBeenCalled();
  await act(async () => button("Hold to dictate").click());
  expect(mocks.voiceStart).toHaveBeenCalledWith(expect.stringContaining("voice-test:"), "local", undefined);
  expect(active).toHaveBeenLastCalledWith(true);
  event("started");
  event("partial", "testing the microphone");
  expect(input().value).toBe("testing the microphone");
  expect(input().readOnly).toBe(true);
  expect(host.querySelector('button[type="submit"]')).toBeNull();
  await act(async () => button("Stop dictating").click());
  expect(release).toHaveBeenCalledTimes(1);
  event("final", "testing my microphone");
  event("closed");
  expect(input().value).toBe("testing my microphone");
  expect(input().readOnly).toBe(false);
  expect(active).toHaveBeenLastCalledWith(false);
  const clear = [...host.querySelectorAll("button")].find(item => item.textContent === "Clear test")!;
  act(() => clear.click());
  expect(input().value).toBe("");
});

it("cancels a test on leaving settings and rejects its late transcript", async () => {
  await act(async () => button("Hold to dictate").click());
  event("started");
  act(() => root.render(null));
  expect(release).toHaveBeenCalledTimes(1);
  expect(mocks.voiceCancel).toHaveBeenCalledWith("microphone-test");
  event("final", "stale words");
  expect(host.textContent).toBe("");
  expect(active).toHaveBeenLastCalledWith(false);
});

it("Escape cancels without applying provisional speech", async () => {
  await act(async () => button("Hold to dictate").click());
  event("started"); event("partial", "discard this");
  act(() => input().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(mocks.voiceCancel).toHaveBeenCalledWith("microphone-test");
  expect(input().value).toBe("");
});

it("Escape cancels from the focused microphone button and restores transcript focus", async () => {
  const mic = button("Hold to dictate");
  mic.focus();
  await act(async () => mic.click());
  event("started"); event("partial", "discard this");
  expect(document.activeElement).toBe(mic);
  act(() => mic.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(mocks.voiceCancel).toHaveBeenCalledWith("microphone-test");
  expect(input().value).toBe("");
  expect(document.activeElement).toBe(input());
});

it.each(["Enter", "Escape"])("pointer start from body makes %s available without selecting the transcript", async key => {
  const mic = button("Hold to dictate");
  mic.setPointerCapture = vi.fn();
  mic.hasPointerCapture = () => true;
  mic.releasePointerCapture = vi.fn();
  expect(document.activeElement).toBe(document.body);
  await act(async () => mic.dispatchEvent(new MouseEvent("pointerdown", { button: 0, bubbles: true, cancelable: true })));
  act(() => mic.dispatchEvent(new MouseEvent("pointerup", { button: 0, bubbles: true })));
  event("started"); event("partial", "microphone words");
  expect(document.activeElement).toBe(input());
  await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
  if (key === "Escape") {
    expect(mocks.voiceCancel).toHaveBeenCalledWith("microphone-test");
    expect(input().value).toBe("");
  } else {
    expect(mocks.voiceStop).toHaveBeenCalledWith("microphone-test");
    expect(host.textContent).toContain("Finishing…");
  }
});

it("preserves Enter activation of Cancel instead of finishing", async () => {
  await act(async () => button("Hold to dictate").click());
  event("started");
  const cancel = button("Cancel dictation");
  cancel.focus();
  const key = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  act(() => cancel.dispatchEvent(key));
  expect(key.defaultPrevented).toBe(false);
  expect(mocks.voiceStop).not.toHaveBeenCalled();
  // jsdom does not synthesize native button activation from Enter.
  act(() => cancel.click());
  expect(mocks.voiceCancel).toHaveBeenCalledWith("microphone-test");
  expect(document.activeElement).toBe(input());
});

it("preserves microphone focus and Space release when starting from the keyboard", async () => {
  const mic = button("Hold to dictate");
  mic.focus();
  await act(async () => mic.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true })));
  event("started");
  expect(document.activeElement).toBe(mic);
  await act(async () => mic.dispatchEvent(new KeyboardEvent("keyup", { key: " ", bubbles: true, cancelable: true })));
  expect(mocks.voiceStop).toHaveBeenCalledTimes(1);
});

it("keeps Escape available after the stopping button loses browser focus", async () => {
  const mic = button("Hold to dictate");
  mic.focus();
  await act(async () => mic.click());
  event("started");
  // Browsers blur a focused button when stopping disables it; jsdom does not.
  mic.blur();
  await act(async () => mic.click());
  expect(mic.disabled).toBe(true);
  expect(document.activeElement).toBe(input());
  act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(mocks.voiceCancel).toHaveBeenCalledWith("microphone-test");
});

it("leaves unrelated settings focus alone when the test finishes", async () => {
  await act(async () => button("Hold to dictate").click());
  event("started");
  const other = document.createElement("button");
  host.append(other);
  other.focus();
  event("closed");
  expect(document.activeElement).toBe(other);
});

it("ignores composing shortcut keys", async () => {
  const mic = button("Hold to dictate");
  mic.focus();
  await act(async () => mic.click());
  event("started");
  for (const key of ["Enter", "Escape"]) {
    const composing = new KeyboardEvent("keydown", { key, isComposing: true, bubbles: true, cancelable: true });
    act(() => mic.dispatchEvent(composing));
    expect(composing.defaultPrevented).toBe(false);
  }
  expect(mocks.voiceStop).not.toHaveBeenCalled();
  expect(mocks.voiceCancel).not.toHaveBeenCalled();
});
