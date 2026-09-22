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
  expect(host.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("50");
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
