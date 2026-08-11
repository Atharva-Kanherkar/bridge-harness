// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
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
    focused: () => document.activeElement?.textContent?.trim() ?? null,
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
    // No Start anywhere on purpose: starting an agent means starting a session,
    // which already has a surface. A Start button here would either strand the
    // user in Settings or need routing this panel does not own.
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
      // Unusable and Bridge's: repair it, or remove it.
      [{ state: "broken", backing: "managed", removable: true }, ["Repair", "Remove"]],
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
    // Attribute selector, not `#id`: React's useId contains characters that are
    // not valid in a CSS id selector.
    expect(view.host.querySelector(`[id="${reason}"]`)?.textContent)
      .toContain("Stop Codex before removing");
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

    await act(async () => {
      release?.({ agentId: "codex", kind: "install", outcome: "installed", status: agent() } as never);
    });
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


  it("a_broken_agent_reads_as_unavailable_rather_than_a_raw_state_string", async () => {
    const view = await render([agent({ state: "broken" })]);
    expect(view.host.querySelector('[data-testid="agent-state-codex"]')?.textContent).toBe("Unavailable");
    await view.unmount();
  });

  it("concurrent_operations_do_not_clear_each_others_busy_state", async () => {
    // One busy slot used to mean the first card to finish cleared the second
    // card's indicator while its RPC was still in flight, re-enabling actions
    // mid-operation.
    const releases: Record<string, (result: never) => void> = {};
    vi.spyOn(bridgeApi, "installManagedAgent").mockImplementation(
      (agentId: string) => new Promise(resolve => { releases[agentId] = resolve as never; }));

    const view = await render([
      agent({ agentId: "codex", label: "Codex", state: "not_installed", backing: "none", removable: false }),
      agent({ agentId: "opencode", label: "OpenCode", state: "not_installed", backing: "none", removable: false }),
    ]);
    const install = (id: string) =>
      view.button("Install", view.host.querySelector(`[data-testid="agent-card-${id}"]`));

    await view.click(install("codex"));
    await view.click(install("opencode"));
    expect(view.host.querySelector('[data-testid="agent-busy-codex"]')).not.toBeNull();
    expect(view.host.querySelector('[data-testid="agent-busy-opencode"]')).not.toBeNull();

    // Finish only codex. opencode must still be busy and must not have its
    // Install button back.
    await act(async () => {
      releases.codex({
        agentId: "codex", kind: "install", outcome: "installed",
        status: agent({ agentId: "codex", label: "Codex" }),
      } as never);
    });
    expect(view.host.querySelector('[data-testid="agent-busy-codex"]')).toBeNull();
    expect(view.host.querySelector('[data-testid="agent-busy-opencode"]')).not.toBeNull();
    expect(install("opencode"), "the still-running card must stay busy").toBeNull();

    await act(async () => {
      releases.opencode({
        agentId: "opencode", kind: "install", outcome: "installed",
        status: agent({ agentId: "opencode", label: "OpenCode" }),
      } as never);
    });
    expect(view.host.querySelector('[data-testid="agent-busy-opencode"]')).toBeNull();
    await view.unmount();
  });

  it("the_confirmation_focuses_keep_not_remove", async () => {
    // A dialog that autofocuses its destructive action turns a stray Enter into
    // an uninstall.
    const uninstall = vi.spyOn(bridgeApi, "uninstallManagedAgent");
    const view = await render([agent()]);
    await view.click(view.button("Remove"));

    expect(view.focused()).toBe("Keep it");
    // And Keep it comes first in tab order for the same reason.
    const order = [...(view.dialog()?.querySelectorAll("button") ?? [])].map(node => node.textContent?.trim());
    expect(order).toEqual(["Keep it", "Remove"]);
    expect(uninstall).not.toHaveBeenCalled();
    await view.unmount();
  });

  it("install_success_applies_the_returned_status", async () => {
    // The external → managed transition, driven by the result rather than a guess.
    const installed = agent({ state: "ready", backing: "managed", removable: true, version: "0.147.0" });
    vi.spyOn(bridgeApi, "installManagedAgent").mockResolvedValue({
      agentId: "codex", kind: "install", outcome: "installed", status: installed,
    });
    const view = await render([agent({ state: "external", backing: "external", removable: false })]);
    await view.click(view.button("Install Bridge-managed copy"));

    expect(view.host.querySelector('[data-testid="agent-state-codex"]')?.textContent).toBe("Ready");
    expect(view.host.querySelector('[data-testid="agent-source-codex"]')?.textContent).toBe("Bridge-managed");
    // Now Bridge's, so removal is offered where it was not before.
    expect(view.button("Remove")).not.toBeNull();
    await view.unmount();
  });

  it("a_list_failure_offers_retry", async () => {
    const list = vi.spyOn(bridgeApi, "listManagedAgents")
      .mockRejectedValueOnce(new Error("The Bridge daemon is not reachable"))
      .mockResolvedValueOnce({ agents: [agent()] });

    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    await act(async () => { root.render(<ManagedAgentsPanel />); });

    expect(host.querySelector('[role="alert"]')?.textContent).toContain("not reachable");
    const retry = [...host.querySelectorAll("button")].find(node => node.textContent?.trim() === "Retry");
    expect(retry, "a failed list must not be a dead end").not.toBeNull();

    await act(async () => { (retry as HTMLButtonElement).click(); });
    expect(list).toHaveBeenCalledTimes(2);
    expect(host.querySelector('[data-testid="agent-card-codex"]')).not.toBeNull();
    expect(host.querySelector('[role="alert"]')).toBeNull();
    await act(async () => { root.unmount(); host.remove(); });
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

describe("ManagedAgentsPanel styling", () => {
  // The review found this panel using `var(--text)`, `var(--text-muted)`, and
  // `var(--danger)` — none of which exist in index.css, so that text rendered
  // with an invalid colour. A visual pass would have caught it; this catches it
  // permanently, which a one-off eyeball does not.
  it("every_colour_class_resolves_to_a_declared_theme_token", async () => {
    // Paths from the project root: import.meta.url is not a file URL under the
    // vite test transform.
    const source = readFileSync("src/components/ManagedAgentsPanel.tsx", "utf8");
    const css = readFileSync("src/index.css", "utf8");

    // Nothing may reach for a raw variable: the theme utilities are the contract.
    expect(source).not.toMatch(/var\(--/);

    // A declared token is not automatically a *text* colour. `--destructive` and
    // `--warning` are surface tokens paired with `*-foreground`, and used as text
    // they resolve near-black on a dark card — visually invisible, which the
    // declared-token check above cannot see. Found by actually looking at it.
    expect(source).not.toMatch(/\btext-destructive\b/);
    expect(source).not.toMatch(/\btext-warning\b/);

    // Tailwind's own palette (text-red-400) is built in; a bare name
    // (text-foreground) has to be a project token.
    const PALETTE = /^(?:red|rose|amber|orange|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|slate|gray|zinc|neutral|stone)-\d{2,3}$/;
    const NOT_A_COLOUR = ["xs", "sm", "base", "lg", "left", "center", "right", "black", "white", "transparent", "current", "inherit"];
    const used = [...source.matchAll(/\b(?:text|bg|border)-([a-z][a-z0-9-]*)/g)]
      .map(match => match[1].replace(/\/.*$/, ""))
      .filter(token => !NOT_A_COLOUR.includes(token) && !PALETTE.test(token));
    expect(used.length, "the panel must use theme colour utilities").toBeGreaterThan(0);

    for (const token of new Set(used)) {
      expect(
        css.includes(`--color-${token}:`) || css.includes(`--${token}:`),
        `${token} is neither a Tailwind palette colour nor a declared theme token — it would render with an invalid colour`,
      ).toBe(true);
    }
  });

  it("renders_without_throwing_in_every_state", async () => {
    // Server-render each state too: a hook-order or undefined-field mistake in a
    // branch the DOM tests happen not to hit still fails here.
    for (const state of ["not_installed", "installed", "ready", "repairable", "broken", "running", "external"]) {
      const markup = renderToStaticMarkup(
        <ManagedAgentsPanel initialAgents={[agent({ state, backing: state === "external" ? "external" : "managed" })]} />,
      );
      expect(markup, state).toContain("Codex");
    }
  });
});
