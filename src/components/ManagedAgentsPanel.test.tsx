// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";
import { ManagedAgentsPanel } from "./ManagedAgentsPanel";

function agent(overrides: Partial<ManagedAgentStatus> = {}): ManagedAgentStatus {
  return {
    agentId: "codex",
    label: "Codex",
    state: "ready",
    backing: "managed",
    removable: true,
    executable: "/managed-runtimes/agents/codex/installations/abc123/payload/bin/codex",
    version: "0.147.0",
    consecutiveFailures: 0,
    ...overrides,
  } as ManagedAgentStatus;
}

/** Render with a fixed agent list, bypassing the initial fetch. */
async function render(agents: ManagedAgentStatus[]) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => { root.render(<ManagedAgentsPanel initialAgents={agents} />); });
  return {
    host,
    text: () => host.textContent ?? "",
    button: (label: string, within?: Element | null) =>
      [...(within ?? host).querySelectorAll("button")]
        .find(node => node.textContent?.trim() === label) ?? null,
    dialog: () => host.querySelector('[data-testid="remove-confirmation"]'),
    buttons: () => [...host.querySelectorAll("button")].map(node => node.textContent?.trim() ?? ""),
    click: async (node: Element | null) => {
      expect(node, "control must exist to be clicked").not.toBeNull();
      await act(async () => { (node as HTMLButtonElement).click(); });
    },
    unmount: () => act(async () => { root.unmount(); host.remove(); }),
  };
}

// React only treats `act` as authoritative when this is set, otherwise every
// render logs a warning that buries real output.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

describe("ManagedAgentsPanel", () => {
  it("renders_one_card_per_built_in_agent", async () => {
    const view = await render([
      agent({ agentId: "claude", label: "Claude Code" }),
      agent({ agentId: "codex", label: "Codex" }),
      agent({ agentId: "opencode", label: "OpenCode", state: "not_installed", backing: "none", removable: false }),
    ]);
    for (const id of ["claude", "codex", "opencode"]) {
      expect(view.host.querySelector(`[data-testid="agent-card-${id}"]`)).not.toBeNull();
    }
    expect(view.text()).toContain("Claude Code");
    expect(view.text()).toContain("OpenCode");
    await view.unmount();
  });

  it("each_state_offers_its_contracted_actions", async () => {
    const cases: Array<[Partial<ManagedAgentStatus>, string[]]> = [
      // Nothing installed: install is the only thing to do.
      [{ state: "not_installed", backing: "none", removable: false, executable: undefined, version: undefined }, ["Install"]],
      // Bridge owns it and it is healthy: removal is available.
      [{ state: "ready", backing: "managed", removable: true }, ["Remove"]],
      [{ state: "installed", backing: "managed", removable: true }, ["Remove"]],
      // Bridge owns drifted bytes: repair, and removal is still Bridge's to offer.
      [{ state: "repairable", backing: "managed", removable: true }, ["Repair", "Remove"]],
      // The user's own copy: offer a managed copy, never removal.
      [{ state: "external", backing: "external", removable: false }, ["Install Bridge-managed copy"]],
    ];
    for (const [overrides, expected] of cases) {
      const view = await render([agent(overrides)]);
      expect(view.buttons(), `state ${overrides.state}`).toEqual(expected);
      await view.unmount();
    }
  });

  it("a_user_managed_runtime_never_offers_removal", async () => {
    for (const backing of ["external", "explicit", "bundled"] as const) {
      const view = await render([agent({ backing, removable: false, state: "external" })]);
      expect(view.button("Remove"), `${backing} must not be removable`).toBeNull();
      // And the card says whose it is, so a working runtime is never presented
      // as Bridge-managed.
      expect(view.host.querySelector('[data-testid="agent-source-codex"]')?.textContent)
        .toMatch(backing === "bundled" ? /Bridge/ : /User-managed/);
      await view.unmount();
    }
  });

  it("removal_is_driven_by_the_removable_field_not_the_state_string", async () => {
    // An unrecognized state must not open the destructive path...
    const unknown = await render([agent({ state: "some-future-state", removable: false })]);
    expect(unknown.button("Remove")).toBeNull();
    expect(unknown.host.querySelector('[data-testid="agent-state-codex"]')?.textContent)
      .toBe("some-future-state");
    await unknown.unmount();

    // ...and the same unrecognized state does when the API says Bridge owns it.
    const owned = await render([agent({ state: "some-future-state", removable: true })]);
    expect(owned.button("Remove")).not.toBeNull();
    await owned.unmount();
  });

  it("a_running_agent_disables_removal_and_says_why", async () => {
    const view = await render([agent({ state: "running", removable: true })]);
    const remove = view.button("Remove") as HTMLButtonElement;
    expect(remove.disabled).toBe(true);
    // The reason is associated with the control, not just printed nearby.
    const reason = remove.getAttribute("aria-describedby");
    expect(reason).not.toBeNull();
    expect(view.host.querySelector(`#${reason}`)?.textContent).toContain("Stop Codex before removing");
    await view.unmount();
  });

  it("removing_requires_confirmation_naming_the_payload", async () => {
    const uninstall = vi.spyOn(bridgeApi, "uninstallManagedAgent");
    const view = await render([agent()]);
    await view.click(view.button("Remove"));

    const dialog = view.host.querySelector('[data-testid="remove-confirmation"]');
    expect(dialog).not.toBeNull();
    // Named precisely: which agent, which version, which path.
    expect(dialog?.textContent).toContain("Codex");
    expect(dialog?.textContent).toContain("0.147.0");
    expect(dialog?.textContent).toContain("/managed-runtimes/agents/codex/installations/abc123/payload/bin/codex");
    expect(dialog?.getAttribute("aria-modal")).toBe("true");

    // Dismissing calls nothing.
    await view.click(view.button("Keep it", view.dialog()));
    expect(view.host.querySelector('[data-testid="remove-confirmation"]')).toBeNull();
    expect(uninstall).not.toHaveBeenCalled();
    await view.unmount();
  });

  it("confirming_removal_calls_the_rpc_once_and_applies_the_returned_status", async () => {
    const removed = agent({ state: "not_installed", backing: "none", removable: false, executable: undefined, version: undefined });
    const uninstall = vi.spyOn(bridgeApi, "uninstallManagedAgent").mockResolvedValue({
      agentId: "codex", kind: "uninstall", outcome: "removed", status: removed,
    });
    const view = await render([agent()]);
    await view.click(view.button("Remove"));
    // The dialog's Remove, not the card's that opened it.
    await view.click(view.button("Remove", view.dialog()));

    expect(uninstall).toHaveBeenCalledTimes(1);
    expect(uninstall).toHaveBeenCalledWith("codex");
    // The new state came from the result, not from an optimistic guess.
    expect(view.host.querySelector('[data-testid="agent-state-codex"]')?.textContent).toBe("Not installed");
    expect(view.button("Remove")).toBeNull();
    expect(view.button("Install")).not.toBeNull();
    await view.unmount();
  });

  it("an_in_flight_operation_shows_busy_without_progress_or_cancel", async () => {
    let release: ((result: never) => void) | undefined;
    vi.spyOn(bridgeApi, "installManagedAgent").mockImplementation(
      () => new Promise(resolve => { release = resolve as never; }));
    const view = await render([agent({ state: "not_installed", backing: "none", removable: false })]);
    await view.click(view.button("Install"));

    const busy = view.host.querySelector('[data-testid="agent-busy-codex"]');
    expect(busy?.textContent).toContain("Installing");
    expect(busy?.getAttribute("role")).toBe("status");
    // Indeterminate on purpose: no percentage to invent, and no cancel to fake.
    expect(view.text()).not.toMatch(/\d+%/);
    expect(view.button("Cancel")).toBeNull();
    expect(view.host.querySelector("progress")).toBeNull();

    release?.({ agentId: "codex", kind: "install", outcome: "installed", status: agent() } as never);
    await view.unmount();
  });

  it("a_failed_operation_shows_its_message_and_leaves_the_card_usable", async () => {
    vi.spyOn(bridgeApi, "installManagedAgent")
      .mockRejectedValue(new Error("codex has no vendor build for this operating system"));
    const view = await render([agent({ state: "not_installed", backing: "none", removable: false })]);
    await view.click(view.button("Install"));

    const alert = view.host.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("no vendor build");
    // Still usable: the failure did not strand the card in a busy state.
    expect(view.host.querySelector('[data-testid="agent-busy-codex"]')).toBeNull();
    expect(view.button("Install")).not.toBeNull();
    await view.unmount();
  });

  it("vendor_guidance_is_shown_verbatim_and_adds_no_credential_controls", async () => {
    const vendorMessage = "Not logged in. Run `codex login` to authenticate with your ChatGPT account.";
    const view = await render([agent({ state: "installed", vendorMessage })]);
    expect(view.host.querySelector('[data-testid="agent-vendor-codex"]')?.textContent).toBe(vendorMessage);
    // Surfaced, not owned: nothing here collects a credential.
    expect(view.host.querySelector("input")).toBeNull();
    expect(view.buttons().join(" ").toLowerCase()).not.toMatch(/log ?in|log ?out|sign ?in|api key/);
    await view.unmount();
  });

  it("the_panel_never_renders_a_credential_control", async () => {
    // Swept across every state, because the boundary has to hold in all of them,
    // not just the one that displays vendor text.
    for (const state of ["not_installed", "installed", "ready", "repairable", "running", "external"]) {
      const view = await render([agent({
        state,
        backing: state === "external" ? "external" : "managed",
        removable: state !== "external" && state !== "not_installed",
        vendorMessage: "Run `codex login`.",
      })]);
      expect(view.host.querySelector("input"), state).toBeNull();
      expect(view.host.querySelector("form"), state).toBeNull();
      expect(view.host.querySelector('[type="password"]'), state).toBeNull();
      expect(view.host.innerHTML.toLowerCase()).not.toContain("apikey");
      await view.unmount();
    }
  });
});
