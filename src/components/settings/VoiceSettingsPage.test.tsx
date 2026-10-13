// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { VoiceLocalStatusResult } from "../../protocol/generated/protocol";
import { VoiceSettingsPage } from "./VoiceSettingsPage";

const base: VoiceLocalStatusResult = {
  state: "notInstalled",
  engineVersion: "1.13.8",
  modelId: "nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25",
  locale: "en-US",
  downloadBytes: 482_197_219,
  installedBytes: 694_157_312,
  downloadedBytes: 0,
  reason: null,
};

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

async function render(status = base, onChanged = vi.fn()) {
  vi.spyOn(bridgeApi, "voiceLocalStatus").mockResolvedValue(status);
  await act(async () => { root.render(<VoiceSettingsPage onError={vi.fn()} onChanged={onChanged} />); });
  return onChanged;
}

function button(label: string): HTMLButtonElement {
  const match = [...host.querySelectorAll("button")].find(item => item.textContent?.includes(label));
  if (!match) throw new Error(`Missing button: ${label}`);
  return match;
}

it("discloses the local model size, language, privacy, and license before setup", async () => {
  await render();
  expect(host.textContent).toContain("English (en-US)");
  expect(host.textContent).toContain("audio stays on this Mac");
  expect(host.textContent).toContain("460 MiB");
  expect(host.textContent).toContain("662 MiB");
  expect(host.textContent).toContain("NVIDIA Open Model License");
  expect(host.querySelector('a[href*="nvidia-open-model-license"]')).not.toBeNull();
  expect(button("Download and install")).toBeTruthy();
});

it("starts only from the explicit install action and reports progress", async () => {
  const setup = vi.spyOn(bridgeApi, "voiceLocalSetup").mockResolvedValue({
    ...base,
    state: "downloadingModel",
    downloadedBytes: 241_098_610,
  });
  await render();
  expect(setup).not.toHaveBeenCalled();
  await act(async () => { button("Download and install").click(); });
  expect(setup).toHaveBeenCalledTimes(1);
  expect(host.textContent).toContain("Downloading model");
  expect(host.querySelector("progress")?.value.toString()).toBe("50");
});

it("removes a ready installation only after confirmation", async () => {
  const changed = vi.fn();
  const remove = vi.spyOn(bridgeApi, "voiceLocalRemove").mockResolvedValue(base);
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  await render({ ...base, state: "ready", downloadedBytes: base.downloadBytes }, changed);
  await act(async () => { button("Remove local model").click(); });
  expect(remove).not.toHaveBeenCalled();
  confirm.mockReturnValue(true);
  await act(async () => { button("Remove local model").click(); });
  expect(remove).toHaveBeenCalledTimes(1);
  expect(changed).toHaveBeenCalled();
});


it("refreshes composer availability when setup immediately reports ready", async () => {
  const changed = vi.fn();
  vi.spyOn(bridgeApi, "voiceLocalSetup").mockResolvedValue({ ...base, state: "ready" });
  await render(base, changed);
  await act(async () => { button("Download and install").click(); });
  expect(changed).toHaveBeenCalledTimes(1);
  expect(button("Remove local model")).toBeTruthy();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

it("keeps installation actions hidden until availability is known and retries status errors", async () => {
  const pending = deferred<VoiceLocalStatusResult>();
  const status = vi.spyOn(bridgeApi, "voiceLocalStatus").mockReturnValueOnce(pending.promise).mockResolvedValue(base);
  await act(async () => root.render(<VoiceSettingsPage onError={vi.fn()} />));
  expect(host.textContent).toContain("Checking…");
  expect(host.textContent).not.toContain("Download and install");
  await act(async () => pending.reject(new Error("offline")));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not check");
  await act(async () => button("Retry status").click());
  expect(status).toHaveBeenCalledTimes(2);
  expect(button("Download and install")).toBeTruthy();
});

it("ignores a stale poll after setup completes and never overlaps slow polls", async () => {
  vi.useFakeTimers();
  try {
    const pending = deferred<VoiceLocalStatusResult>();
    const initial = { ...base, state: "downloadingModel" as const };
    const status = vi.spyOn(bridgeApi, "voiceLocalStatus").mockResolvedValueOnce(initial).mockReturnValueOnce(pending.promise);
    await act(async () => root.render(<VoiceSettingsPage onError={vi.fn()} />));
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(status).toHaveBeenCalledTimes(2);
    await act(async () => pending.resolve({ ...base, state: "ready" }));
    expect(host.textContent).toContain("Try your microphone");
    expect(host.querySelector("progress")).toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(1500));
    expect(status).toHaveBeenCalledTimes(2);
  } finally { vi.useRealTimers(); }
});

it("ignores an old retry response after a newer result removes the installation", async () => {
  vi.useFakeTimers();
  try {
    const pending = deferred<VoiceLocalStatusResult>();
    const changed = vi.fn();
    vi.spyOn(bridgeApi, "voiceLocalStatus").mockRejectedValueOnce(new Error("offline")).mockReturnValueOnce(pending.promise).mockResolvedValue({ ...base, state: "ready" });
    const remove = vi.spyOn(bridgeApi, "voiceLocalRemove").mockResolvedValue(base);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await act(async () => root.render(<VoiceSettingsPage onError={vi.fn()} onChanged={changed} />));
    await act(async () => button("Retry status").click());
    await act(async () => button("Retry status").click());
    await act(async () => button("Remove local model").click());
    expect(remove).toHaveBeenCalledTimes(1);
    await act(async () => pending.resolve({ ...base, state: "ready" }));
    expect(button("Download and install")).toBeTruthy();
    expect(host.textContent).not.toContain("Try your microphone");
    expect(changed).toHaveBeenCalledTimes(2);
  } finally { vi.useRealTimers(); }
});

it("refreshes composer availability when installed speech is discovered on opening settings", async () => {
  const changed = await render({ ...base, state: "ready" });
  expect(changed).toHaveBeenCalledTimes(1);
  expect(host.textContent).toContain("Try your microphone");
  expect(host.textContent).not.toContain("sherpa-onnx");
  await act(async () => button("Model details").click());
  expect(host.textContent).toContain("sherpa-onnx 1.13.8");
});
