// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { UpdateToast } from "./UpdateToast";
import type { UpdateInfo } from "../updater";

const development = vi.hoisted(() => ({ build: true }));
vi.mock("../updater", async importOriginal => ({
  ...(await importOriginal<typeof import("../updater")>()),
  isDevelopmentBuild: () => development.build,
}));

let root: Root | undefined;
let host: HTMLDivElement | undefined;

const update: UpdateInfo = { version: "0.42.0", currentVersion: "0.41.2", body: null, channel: "stable" };

async function mount(ui: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root?.render(ui); });
}

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove();
  vi.useRealTimers();
  development.build = true;
});

const installButton = () => [...host!.querySelectorAll("button")].find(button => /Install|Retry/.test(button.textContent ?? "")) as HTMLButtonElement | undefined;

describe("UpdateToast", () => {
  it("tells a development build to install from a packaged app", async () => {
    await mount(<UpdateToast update={update} onInstall={async () => undefined} onDismiss={() => undefined} />);
    expect(host!.textContent).toContain("Install updates from a packaged Bridge app.");
    expect(installButton()).toBeUndefined();
  });

  it("sits in the stack's flow and stays until dismissed", async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    await mount(<UpdateToast update={update} onInstall={async () => undefined} onDismiss={onDismiss} />);
    expect(host!.innerHTML).not.toContain("fixed");
    expect(host!.textContent).toContain("Bridge 0.42.0 is available");
    await act(async () => { vi.advanceTimersByTime(600_000); });
    expect(onDismiss).not.toHaveBeenCalled();
    (host!.querySelector('button[aria-label="Dismiss notification"]') as HTMLButtonElement).click();
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("installs, and shows the error line with a retry on failure", async () => {
    // A packaged build: development builds show a note instead of the button.
    development.build = false;
    const onInstall = vi.fn().mockRejectedValueOnce(new Error("network")).mockResolvedValueOnce(undefined);
    await mount(<UpdateToast update={update} onInstall={onInstall} onDismiss={() => undefined} />);
    await act(async () => { installButton()!.click(); });
    expect(onInstall).toHaveBeenCalledWith(update);
    expect(host!.textContent).toContain("Update failed. Check your connection and try again.");
    expect(installButton()!.textContent).toBe("Retry");
    await act(async () => { installButton()!.click(); });
    expect(onInstall).toHaveBeenCalledTimes(2);
  });
});
