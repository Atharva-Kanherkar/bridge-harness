// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";
import type { AdapterDescriptor } from "../types";
import { ManagedAgentDetail, ManagedAgentsPanel, useManagedAgents } from "./ManagedAgentsPanel";

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

function adapter(authState: AdapterDescriptor["authState"] = "signed_in"): AdapterDescriptor {
  return {
    id: "codex", label: "Codex", available: true, authState, version: "0.147.0",
    capabilities: [], models: [], unavailableReason: null,
  };
}

/** The runtime block of a harness detail page, over one fixed agent list. */
function Detail({ agents, agentId }: { agents: ManagedAgentStatus[]; agentId: string }) {
  const state = useManagedAgents(agents);
  return <>
    <ManagedAgentDetail state={state} agentId={agentId} />
    {state.confirmation}
  </>;
}

function view(host: HTMLElement, root: { unmount: () => void }) {
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

/** The list, with a fixed agent list, bypassing the initial fetch. */
async function render(
  agents: ManagedAgentStatus[],
  onOpen?: (agentId: string) => void,
  adapters: AdapterDescriptor[] = [adapter()],
  onChanged?: () => void,
) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => { root.render(<ManagedAgentsPanel initialAgents={agents} adapters={adapters} onOpen={onOpen} onChanged={onChanged} />); });
  return view(host, root);
}

/** One agent's detail block, which is where the destructive actions live. */
async function renderDetail(agents: ManagedAgentStatus[], agentId = agents[0].agentId) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => { root.render(<Detail agents={agents} agentId={agentId} />); });
  return view(host, root);
}

// React only treats `act` as authoritative when this is set, otherwise every
// render logs a warning that buries real output.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

describe("the runtime list", () => {
  it("renders_one_row_per_built_in_agent", async () => {
    const view = await render([
      agent({ agentId: "claude", label: "Claude Code" }),
      agent({ agentId: "codex", label: "Codex" }),
      agent({ agentId: "cursor", label: "Cursor", state: "external", backing: "external", removable: false }),
      agent({ agentId: "opencode", label: "OpenCode", state: "not_installed", backing: "none", removable: false }),
    ]);
    for (const id of ["claude", "codex", "cursor", "opencode"]) {
      expect(view.host.querySelector(`[data-testid="agent-card-${id}"]`)).not.toBeNull();
    }
    expect(view.text()).toContain("Claude Code");
    expect(view.text()).toContain("Cursor");
    expect(view.text()).toContain("OpenCode");
    await view.unmount();
  });

  // Installed and Available answer different questions, so they are different
  // groups rather than one 3800px scroll.
  it("separates what Bridge can start from what it can install", async () => {
    const view = await render([
      agent({ agentId: "codex", label: "Codex" }),
      agent({ agentId: "opencode", label: "OpenCode", state: "not_installed", backing: "none", removable: false }),
    ]);
    expect(view.text()).toContain("Installed");
    expect(view.text()).toContain("Available");
    await view.unmount();
  });

  it("a_list_row_offers_install_or_repair_and_nothing_destructive", async () => {
    // No Start anywhere on purpose: starting an agent means starting a session,
    // which already has a surface. Remove lives on the detail page, because a
    // destructive action belongs on the page about the thing.
    const cases: Array<[Partial<ManagedAgentStatus>, string[]]> = [
      [{ state: "not_installed", backing: "none", removable: false, executable: undefined, version: undefined }, ["Install"]],
      [{ state: "ready", backing: "managed", removable: true }, []],
      [{ state: "installed", backing: "managed", removable: true }, []],
      [{ state: "repairable", backing: "managed", removable: true }, ["Repair"]],
      [{ state: "external", backing: "external", removable: false }, []],
      [{ state: "broken", backing: "managed", removable: true }, ["Repair"]],
    ];
    for (const [overrides, expected] of cases) {
      const view = await render([agent(overrides)]);
      expect(view.buttons(), `state ${overrides.state}`).toEqual(expected);
      await view.unmount();
    }
  });

  it("shows authentication actions only when the provider is known to be signed out", async () => {
    const signedIn = await render([agent()]);
    expect(signedIn.text()).toContain("Signed in");
    expect(signedIn.button("Sign in")).toBeNull();
    await signedIn.unmount();

    const signedOut = await render([agent()], undefined, [adapter("signed_out")]);
    expect(signedOut.button("Sign in")).not.toBeNull();
    expect(signedOut.text()).not.toContain("Signed in");
    await signedOut.unmount();

    const unknown = await render([agent()], undefined, [adapter("unknown")]);
    expect(unknown.text()).toContain("Sign-in status unknown");
    expect(unknown.button("Sign in")).toBeNull();
    await unknown.unmount();
  });

  it("refreshes authoritative auth state when a sign-in pane closes", async () => {
    const changed = vi.fn();
    vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "codex" });
    const view = await render([agent()], undefined, [adapter("signed_out")], changed);
    await view.click(view.button("Sign in"));
    await view.click(view.button("Cancel"));
    expect(changed).toHaveBeenCalledTimes(1);
    await view.unmount();
  });

  it("a row opens its harness detail page", async () => {
    const onOpen = vi.fn();
    const view = await render([agent()], onOpen);
    await view.click(view.host.querySelector('[aria-label="Configure Codex"]'));
    expect(onOpen).toHaveBeenCalledWith("codex");
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

  it("concurrent_operations_do_not_clear_each_others_busy_state", async () => {
    // One busy slot used to mean the first row to finish cleared the second
    // row's indicator while its RPC was still in flight, re-enabling actions
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
    expect(install("opencode"), "the still-running row must stay busy").toBeNull();

    await act(async () => {
      releases.opencode({
        agentId: "opencode", kind: "install", outcome: "installed",
        status: agent({ agentId: "opencode", label: "OpenCode" }),
      } as never);
    });
    expect(view.host.querySelector('[data-testid="agent-busy-opencode"]')).toBeNull();
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

  it("a_failed_operation_shows_its_message_and_leaves_the_row_usable", async () => {
    vi.spyOn(bridgeApi, "installManagedAgent")
      .mockRejectedValue(new Error("codex has no vendor build for this operating system"));
    const view = await render([agent({ state: "not_installed", backing: "none", removable: false })]);
    await view.click(view.button("Install"));

    const alert = view.host.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("no vendor build");
    // Still usable: the failure did not strand the row in a busy state.
    expect(view.host.querySelector('[data-testid="agent-busy-codex"]')).toBeNull();
    expect(view.button("Install")).not.toBeNull();
    await view.unmount();
  });

  it("a_broken_agent_reads_as_unavailable_rather_than_a_raw_state_string", async () => {
    const view = await render([agent({ state: "broken" })]);
    expect(view.host.querySelector('[data-testid="agent-state-codex"]')?.textContent).toBe("Unavailable");
    await view.unmount();
  });
});

describe("the runtime block of a harness detail page", () => {
  it("each_state_offers_its_contracted_actions", async () => {
    const cases: Array<[Partial<ManagedAgentStatus>, string[]]> = [
      // Nothing installed: install is the only thing to do.
      [{ state: "not_installed", backing: "none", removable: false, executable: undefined, version: undefined }, ["Install"]],
      // Bridge owns it and it is healthy: removal is available.
      [{ state: "ready", backing: "managed", removable: true }, ["Remove"]],
      [{ state: "installed", backing: "managed", removable: true }, ["Remove"]],
      // Bridge owns drifted bytes: repair, and removal is still Bridge's to offer.
      [{ state: "repairable", backing: "managed", removable: true }, ["Repair", "Remove"]],
      // The user's own copy: it already works, so the only affordance is a quiet
      // opt-in to a managed copy, and never a Remove.
      [{ state: "external", backing: "external", removable: false }, ["Let Bridge manage its own copy"]],
      // Unusable and Bridge's: repair it, or remove it.
      [{ state: "broken", backing: "managed", removable: true }, ["Repair", "Remove"]],
    ];
    for (const [overrides, expected] of cases) {
      const view = await renderDetail([agent(overrides)]);
      expect(view.buttons(), `state ${overrides.state}`).toEqual(expected);
      await view.unmount();
    }
  });

  it("a_user_managed_runtime_never_offers_removal", async () => {
    for (const backing of ["external", "explicit", "bundled"] as const) {
      const view = await renderDetail([agent({ backing, removable: false, state: "external" })]);
      expect(view.button("Remove"), `${backing} must not be removable`).toBeNull();
      // And it says whose it is, so a working runtime is never presented as
      // Bridge-managed.
      expect(view.host.querySelector('[data-testid="agent-source-codex"]')?.textContent)
        .toMatch(backing === "bundled" ? /Bridge/ : /Your own install/);
      await view.unmount();
    }
  });

  it("a_working_user_install_reads_as_settled_not_pending", async () => {
    // It used to show "Install Bridge-managed copy" as its only button, which
    // reads as "this needs installing" for an agent the user can already chat
    // with. A working install states that it works and needs nothing.
    const view = await renderDetail([agent({ state: "external", backing: "external", removable: false })]);
    expect(view.text()).toContain("Working");
    expect(view.text()).toContain("Nothing to install");
    expect(view.text()).toContain("Your own install");
    expect(view.buttons()).toEqual(["Let Bridge manage its own copy"]);
    await view.unmount();
  });

  it("removal_is_driven_by_the_removable_field_not_the_state_string", async () => {
    // An unrecognized state must not open the destructive path...
    const unknown = await renderDetail([agent({ state: "some-future-state", removable: false })]);
    expect(unknown.button("Remove")).toBeNull();
    expect(unknown.host.querySelector('[data-testid="agent-state-codex"]')?.textContent)
      .toBe("some-future-state");
    await unknown.unmount();

    // ...and the same unrecognized state does when the API says Bridge owns it.
    const owned = await renderDetail([agent({ state: "some-future-state", removable: true })]);
    expect(owned.button("Remove")).not.toBeNull();
    await owned.unmount();
  });

  it("a_running_agent_disables_removal_and_says_why", async () => {
    const view = await renderDetail([agent({ state: "running", removable: true })]);
    const remove = view.button("Remove") as HTMLButtonElement;
    expect(remove.disabled).toBe(true);
    expect(view.text()).toContain("Stop Codex before removing it");
    await view.unmount();
  });

  it("removing_requires_confirmation_naming_the_payload", async () => {
    const uninstall = vi.spyOn(bridgeApi, "uninstallManagedAgent");
    const view = await renderDetail([agent()]);
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
    const view = await renderDetail([agent()]);
    await view.click(view.button("Remove"));
    // The dialog's Remove, not the one that opened it.
    await view.click(view.button("Remove", view.dialog()));

    expect(uninstall).toHaveBeenCalledTimes(1);
    expect(uninstall).toHaveBeenCalledWith("codex");
    // The new state came from the result, not from an optimistic guess.
    expect(view.host.querySelector('[data-testid="agent-state-codex"]')?.textContent).toBe("Not installed");
    expect(view.button("Remove")).toBeNull();
    expect(view.button("Install")).not.toBeNull();
    await view.unmount();
  });

  it("the_confirmation_focuses_keep_not_remove", async () => {
    // A dialog that autofocuses its destructive action turns a stray Enter into
    // an uninstall.
    const uninstall = vi.spyOn(bridgeApi, "uninstallManagedAgent");
    const view = await renderDetail([agent()]);
    await view.click(view.button("Remove"));

    expect(view.focused()).toBe("Keep it");
    // And Keep it comes first in tab order for the same reason.
    const order = [...(view.dialog()?.querySelectorAll("button") ?? [])].map(node => node.textContent?.trim());
    expect(order).toEqual(["Keep it", "Remove"]);
    expect(uninstall).not.toHaveBeenCalled();
    await view.unmount();
  });

  it("install_success_applies_the_returned_status", async () => {
    // The external to managed transition, driven by the result rather than a guess.
    const installed = agent({ state: "ready", backing: "managed", removable: true, version: "0.147.0" });
    vi.spyOn(bridgeApi, "installManagedAgent").mockResolvedValue({
      agentId: "codex", kind: "install", outcome: "installed", status: installed,
    });
    const view = await renderDetail([agent({ state: "external", backing: "external", removable: false })]);
    await view.click(view.button("Let Bridge manage its own copy"));

    expect(view.host.querySelector('[data-testid="agent-state-codex"]')?.textContent).toBe("Ready");
    expect(view.host.querySelector('[data-testid="agent-source-codex"]')?.textContent).toContain("Bridge-managed");
    // Now Bridge's, so removal is offered where it was not before.
    expect(view.button("Remove")).not.toBeNull();
    await view.unmount();
  });

  it("vendor_guidance_is_shown_verbatim_and_adds_no_credential_controls", async () => {
    const vendorMessage = "Not logged in. Run `codex login` to authenticate with your ChatGPT account.";
    const view = await renderDetail([agent({ state: "installed", vendorMessage })]);
    expect(view.host.querySelector('[data-testid="agent-vendor-codex"]')?.textContent).toBe(vendorMessage);
    // Surfaced, not owned: nothing here collects a credential.
    expect(view.host.querySelector("input")).toBeNull();
    expect(view.buttons().join(" ").toLowerCase()).not.toMatch(/log ?in|log ?out|sign ?in|api key/);
    await view.unmount();
  });

  it("never_renders_a_credential_control", async () => {
    // Swept across every state, because the boundary has to hold in all of them,
    // not just the one that displays vendor text.
    for (const state of ["not_installed", "installed", "ready", "repairable", "running", "external"]) {
      const view = await renderDetail([agent({
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

    // A declared token is still not automatically a *text* colour, but Graphite
    // & Paper swapped which half of each pair is safe: `--destructive` and
    // `--warning` are now status ink, legible on any resting surface, while
    // `*-foreground` is the paper laid on top of that ink. Used as text on a
    // card a `*-foreground` token resolves to near-paper and disappears — the
    // mirror image of the bug this originally caught, and just as invisible to
    // the declared-token check below.
    expect(source).not.toMatch(/\btext-destructive-foreground\b/);
    expect(source).not.toMatch(/\btext-warning-foreground\b/);

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
      const list = renderToStaticMarkup(
        <ManagedAgentsPanel initialAgents={[agent({ state, backing: state === "external" ? "external" : "managed" })]} />,
      );
      expect(list, state).toContain("Codex");
      const detail = renderToStaticMarkup(
        <Detail agents={[agent({ state, backing: state === "external" ? "external" : "managed" })]} agentId="codex" />,
      );
      expect(detail, state).toContain("Codex");
    }
  });
});
