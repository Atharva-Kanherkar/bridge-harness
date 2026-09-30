// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { ClonesPage } from "./ClonesPage";

// Contract: testing/feat-dock-clone.md §4. The api is NOT mocked here: the
// page reads and writes through the mock api's own reader and writer, so a value
// that survives a remount proves the round trip rather than a stubbed one.

let host: HTMLDivElement;
let root: Root;
const onError = vi.fn();

async function mount() {
  await act(async () => {
    root.render(<ClonesPage onError={onError} />);
  });
}
const unmount = async () => { await act(async () => root.unmount()); };

const trigger = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
/** Open a select and pick an option by its visible label. */
async function pick(label: string, option: string) {
  await act(async () => trigger(label).click());
  const choice = [...document.querySelectorAll<HTMLElement>(`[role="listbox"][aria-label="${label}"] [role="option"]`)]
    .find(node => node.textContent?.includes(option));
  if (!choice) throw new Error(`no "${option}" option in the ${label} listbox`);
  await act(async () => {
    choice.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    choice.click();
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  onError.mockReset();
});

afterEach(async () => {
  await unmount();
  host.remove();
  vi.restoreAllMocks();
  // The mock api keeps its state for the life of the module.
  await bridgeApi.writeCloneSettings({ defaultSignInPath: "import", ttlMinutes: 30 });
});

describe("Clones settings", () => {
  it("starts from import and thirty minutes", async () => {
    await mount();
    expect(trigger("Default sign-in path").textContent).toContain("Import from my browser");
    expect(trigger("Clone time to live").textContent).toContain("30 minutes");
    expect(await bridgeApi.readCloneSettings()).toEqual({ connected: true, settings: { defaultSignInPath: "import", ttlMinutes: 30 } });
  });

  it("writes each change through the api and reads it back after a remount", async () => {
    await mount();
    await pick("Default sign-in path", "Sign in inside the clone");
    await pick("Clone time to live", "1 hour");

    // The writer reached the store: the reader sees both values.
    expect((await bridgeApi.readCloneSettings()).settings).toEqual({ defaultSignInPath: "sign_in_inside", ttlMinutes: 60 });
    expect(trigger("Default sign-in path").textContent).toContain("Sign in inside the clone");
    expect(host.textContent).toContain("Saved");

    // And the page's own reader restores them from a cold mount.
    await unmount();
    root = createRoot(host);
    await mount();
    expect(trigger("Default sign-in path").textContent).toContain("Sign in inside the clone");
    expect(trigger("Clone time to live").textContent).toContain("1 hour");
    expect(onError).not.toHaveBeenCalled();
  });

  it("keeps a stored lifetime the list does not offer", async () => {
    await bridgeApi.writeCloneSettings({ defaultSignInPath: "import", ttlMinutes: 45 });
    await mount();
    expect(trigger("Clone time to live").textContent).toContain("45 minutes");
  });

  it("reports a failed write and keeps showing the stored value", async () => {
    vi.spyOn(bridgeApi, "writeCloneSettings").mockRejectedValue(new Error("write refused"));
    await mount();
    await pick("Clone time to live", "2 hours");
    expect(onError).toHaveBeenCalledWith("write refused");
    expect(trigger("Clone time to live").textContent).toContain("30 minutes");
  });

  it("says so, and disables both controls, when the build has no clone backend", async () => {
    vi.spyOn(bridgeApi, "readCloneSettings").mockResolvedValue({ connected: false, settings: { defaultSignInPath: "import", ttlMinutes: 30 } });
    await mount();
    expect(host.textContent).toContain("not connected to the runtime");
    for (const label of ["Default sign-in path", "Clone time to live"]) {
      const control = trigger(label);
      expect(control.hasAttribute("disabled") || control.getAttribute("aria-disabled") === "true").toBe(true);
    }
  });
});
