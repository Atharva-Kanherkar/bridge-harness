// @vitest-environment jsdom
// The composer's access control: the approval mode is chosen where the work
// happens, in plain words, with no warning tint — a mode is a setting, not an
// alarm.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AccessControl, accessModeOf } from "./AccessControl";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("AccessControl", () => {
  it("names the current mode and offers the other", () => {
    const onChange = vi.fn();
    act(() => { root.render(<AccessControl policy={{ autoApproveProviderPermissions: true }} onChange={onChange} />); });
    const trigger = container.querySelector<HTMLButtonElement>("button")!;
    expect(trigger.textContent).toContain("Full access");
    expect(trigger.className).not.toMatch(/warning/);
    act(() => { trigger.click(); });
    const items = [...document.querySelectorAll<HTMLElement>('[role="menuitemradio"]')];
    expect(items.map(item => item.textContent)).toEqual(expect.arrayContaining([expect.stringContaining("Full access"), expect.stringContaining("User approval")]));
    const ask = items.find(item => item.textContent?.includes("User approval"))!;
    act(() => { ask.click(); });
    expect(onChange).toHaveBeenCalledWith("ask");
  });

  it("reads 'User approval' when Bridge asks first, and does not fire for the current mode", () => {
    const onChange = vi.fn();
    act(() => { root.render(<AccessControl policy={{ autoApproveProviderPermissions: false }} onChange={onChange} />); });
    const trigger = container.querySelector<HTMLButtonElement>("button")!;
    expect(trigger.textContent).toContain("User approval");
    act(() => { trigger.click(); });
    const same = [...document.querySelectorAll<HTMLElement>('[role="menuitemradio"]')].find(item => item.textContent?.includes("User approval"))!;
    act(() => { same.click(); });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("waits, disabled, until the policy has loaded", () => {
    act(() => { root.render(<AccessControl policy={undefined} onChange={() => undefined} />); });
    expect(container.querySelector<HTMLButtonElement>("button")!.disabled).toBe(true);
    expect(accessModeOf(undefined)).toBeNull();
    expect(accessModeOf({})).toBe("ask");
  });
});
