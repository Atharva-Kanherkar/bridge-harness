// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { PermissionsSection } from "./SettingsScreen";
import type { BridgeEvent, PermissionPolicy } from "../types";

const policy = (bypassAll: boolean): PermissionPolicy => ({ bypassAll, updatedAt: "2026-08-21T10:00:00Z" });
const ledgerRow = (id: number, body: string): BridgeEvent => ({
  id, source: "approval", kind: "approval.auto_allowed", entityId: "chat", body,
  createdAt: "2026-08-21T10:00:00Z",
});

async function mount(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, unmount: () => act(async () => root.unmount()) };
}

describe("PermissionsSection", () => {
  it("shows the switch off by default and does not claim to be bypassing", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("OFF");
    await unmount();
  });

  /// The issue makes this copy part of the contract: a switch that claims to
  /// silence everything and then still prompts has to say where, up front.
  it("names both gates that keep asking either way", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(true)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    expect(container.textContent).toContain("Worker write scope");
    expect(container.textContent).toContain("Browser outward effects");
    expect(container.textContent).toContain("These keep asking either way");
    await unmount();
  });

  it("reports the requested flip rather than applying it locally", async () => {
    // The switch is a security control: it must render from what the host
    // stored, so the component asks and re-renders from the returned policy.
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy={false} onChange={onChange} />,
    );
    await act(async () => container.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
    expect(onChange).toHaveBeenCalledWith({ bypassAll: true, updatedAt: "2026-08-21T10:00:00Z" });
    // Still off in the DOM: nothing changed until the host says so.
    expect(container.querySelector('[role="switch"]')!.getAttribute("aria-checked")).toBe("false");
    await unmount();
  });

  it("lists recent auto-approvals so a bypassed approval is auditable", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection
        policy={policy(true)}
        autoApprovals={[ledgerRow(1, "bypass_all granted approval 12"), ledgerRow(2, "bypass_all granted approval 15")]}
        busy={false}
        onChange={() => undefined}
      />,
    );
    expect(container.textContent).toContain("bypass_all granted approval 12");
    expect(container.textContent).toContain("bypass_all granted approval 15");
    expect(container.textContent).not.toContain("Nothing has been auto-approved yet");
    await unmount();
  });

  it("says so plainly when nothing has been auto-approved", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(true)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    expect(container.textContent).toContain("Nothing has been auto-approved yet");
    await unmount();
  });

  it("cannot be flipped while a save is in flight", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy onChange={onChange} />,
    );
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.disabled).toBe(true);
    await act(async () => toggle.click());
    expect(onChange).not.toHaveBeenCalled();
    await unmount();
  });
});
