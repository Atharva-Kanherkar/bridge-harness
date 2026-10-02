// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { UpdateInfo } from "../../updater";
import { checkForUpdate, UpdateCheckSupersededError } from "../../updater";
import { UpdatesPage } from "./UpdatesPage";

vi.mock("../../updater", async importOriginal => ({
  ...await importOriginal<typeof import("../../updater")>(),
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


it("does not display a feed error for a superseded check", async () => {
  vi.mocked(checkForUpdate).mockRejectedValueOnce(new UpdateCheckSupersededError());
  await act(async () => root.render(<UpdatesPage onUpdate={() => undefined} />));
  await act(async () => container.querySelector<HTMLButtonElement>("button:not([role])")?.click());
  expect(container.textContent).not.toContain("Could not check");
  expect(container.textContent).not.toContain("up to date");
  expect(container.textContent).toContain("Check now");
});

it("does not publish a pending check after leaving the Updates page", async () => {
  let resolve!: (update: UpdateInfo) => void;
  vi.mocked(checkForUpdate).mockReturnValueOnce(new Promise(done => { resolve = done; }));
  const onUpdate = vi.fn();
  await act(async () => root.render(<UpdatesPage onUpdate={onUpdate} />));
  await act(async () => container.querySelector<HTMLButtonElement>("button:not([role])")?.click());
  expect(onUpdate).toHaveBeenCalledWith(undefined);
  onUpdate.mockClear();
  await act(async () => root.render(<div />));
  await act(async () => resolve({ version: "0.5.11", currentVersion: "0.5.10", body: null, channel: "beta" }));
  expect(onUpdate).not.toHaveBeenCalled();
});
