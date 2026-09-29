// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { UpdateInfo } from "../../updater";
import { checkForUpdate } from "../../updater";
import { UpdatesPage } from "./UpdatesPage";

vi.mock("../../updater", () => ({
  getUpdateChannel: () => "beta",
  setUpdateChannel: vi.fn(),
  checkForUpdate: vi.fn(),
}));

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

it("replaces an old feed error when a later check finds an update", async () => {
  vi.mocked(checkForUpdate).mockRejectedValueOnce("feed unavailable");
  await act(async () => root.render(<UpdatesPage onUpdate={() => undefined} />));
  await act(async () => container.querySelector<HTMLButtonElement>("button:not([role])")?.click());
  expect(container.textContent).toContain("Could not check for updates: feed unavailable");

  const availableUpdate: UpdateInfo = {
    version: "0.5.11-nightly.20260928", currentVersion: "0.5.10", body: null, channel: "beta",
  };
  await act(async () => root.render(<UpdatesPage availableUpdate={availableUpdate} onUpdate={() => undefined} />));
  expect(container.textContent).toContain("Bridge 0.5.11-nightly.20260928 is available on this channel.");
  expect(container.textContent).not.toContain("feed unavailable");
});
